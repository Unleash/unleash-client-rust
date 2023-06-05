// Copyright 2020 Cognite AS
//! The primary interface for users of the library.
use std::collections::hash_map::HashMap;
use std::default::Default;
use std::fmt::{self, Debug, Display};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use chrono::Utc;
use enum_map::{EnumArray, EnumMap};
use futures_timer::Delay;
use log::{debug, trace, warn};
use rand::Rng;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::api::{self, Feature, Features, Metrics, MetricsBucket, Registration};
use crate::context::Context;
use crate::http::{HttpClient, HTTP};
use crate::strategy;

// ----------------- Variant

/// Variant is returned from `Client.get_variant` and is a cut down and
/// ergonomic version of `api.get_variant`
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Variant {
    pub name: String,
    pub payload: HashMap<String, String>,
    pub enabled: bool,
}

impl From<&CachedVariant> for Variant {
    fn from(variant: &CachedVariant) -> Self {
        Self {
            name: variant.name.clone(),
            payload: variant.payload.as_ref().cloned().unwrap_or_default(),
            enabled: true,
        }
    }
}

impl Variant {
    fn disabled() -> Self {
        Self {
            name: "disabled".into(),
            ..Default::default()
        }
    }
}

// ----------------- ClientBuilder

pub struct ClientBuilder {
    disable_metric_submission: bool,
    enable_str_features: bool,
    interval: u64,
    strategies: HashMap<String, strategy::Strategy>,
}

impl ClientBuilder {
    pub fn into_client<F, C>(
        self,
        api_url: &str,
        app_name: &str,
        instance_id: &str,
        authorization: Option<String>,
    ) -> Result<Client<F, C>, C::Error>
    where
        F: EnumArray<CachedFeature> + Debug + DeserializeOwned + Serialize,
        C: HttpClient + Default,
    {
        Ok(Client {
            api_url: api_url.into(),
            app_name: app_name.into(),
            disable_metric_submission: self.disable_metric_submission,
            enable_str_features: self.enable_str_features,
            instance_id: instance_id.into(),
            interval: self.interval,
            polling: AtomicBool::new(false),
            http: HTTP::new(app_name.into(), instance_id.into(), authorization)?,
            cached_state: ArcSwapOption::from(None),
            strategies: Mutex::new(self.strategies),
        })
    }

    pub fn disable_metric_submission(mut self) -> Self {
        self.disable_metric_submission = true;
        self
    }

    pub fn enable_string_features(mut self) -> Self {
        self.enable_str_features = true;
        self
    }

    pub fn interval(mut self, interval: u64) -> Self {
        self.interval = interval;
        self
    }

    pub fn strategy(mut self, name: &str, strategy: strategy::Strategy) -> Self {
        self.strategies.insert(name.into(), strategy);
        self
    }
}

impl Default for ClientBuilder {
    fn default() -> ClientBuilder {
        let result = ClientBuilder {
            disable_metric_submission: false,
            enable_str_features: false,
            interval: 15000,
            strategies: Default::default(),
        };
        result
            .strategy("default", Box::new(&strategy::default))
            .strategy("applicationHostname", Box::new(&strategy::hostname))
            .strategy("default", Box::new(&strategy::default))
            .strategy("gradualRolloutRandom", Box::new(&strategy::random))
            .strategy("gradualRolloutSessionId", Box::new(&strategy::session_id))
            .strategy("gradualRolloutUserId", Box::new(&strategy::user_id))
            .strategy("remoteAddress", Box::new(&strategy::remote_address))
            .strategy("userWithId", Box::new(&strategy::user_with_id))
            .strategy("flexibleRollout", Box::new(&strategy::flexible_rollout))
    }
}

#[derive(Default)]
pub struct CachedFeature {
    pub strategies: Vec<strategy::Evaluate>,
    // unknown features are tracked for metrics (so the server can see that they
    // are being used). They require specific logic (see is_enabled).
    known: bool,
    // disabled features behaviour differently to empty strategies, so we carry
    // this field across.
    feature_disabled: bool,
    // Tracks metrics during a refresh interval. If the AtomicBool updates show
    // to be a contention point then thread-sharded counters with a gather phase
    // on submission will be the next logical progression.
    enabled: AtomicU64,
    disabled: AtomicU64,
    disabled_variant_count: AtomicU64,
    // Variants for use with get_variant
    variants: Vec<CachedVariant>,
}

impl CachedFeature {
    #[allow(dead_code)]
    fn variant_metrics(&self) -> HashMap<String, u64> {
        self.variants
            .iter()
            .map(|variant| (variant.name.clone(), variant.count.load(Ordering::Relaxed)))
            .chain([(
                "disabled".into(),
                self.disabled_variant_count.load(Ordering::Relaxed),
            )])
            .collect()
    }
}

#[derive(Default)]
pub struct CachedVariant {
    count: AtomicU64,
    name: String,
    weight: u8,
    payload: Option<HashMap<String, String>>,
    overrides: Option<Vec<api::VariantOverride>>,
}

impl Clone for CachedVariant {
    fn clone(&self) -> Self {
        Self {
            count: AtomicU64::new(self.count.load(Ordering::Relaxed)),
            name: self.name.clone(),
            weight: self.weight,
            payload: self.payload.clone(),
            overrides: self.overrides.clone(),
        }
    }
}

impl From<api::Variant> for CachedVariant {
    fn from(value: api::Variant) -> Self {
        CachedVariant {
            count: AtomicU64::new(0),
            name: value.name,
            weight: value.weight,
            payload: value.payload,
            overrides: value.overrides,
        }
    }
}

pub struct CachedState<F>
where
    F: EnumArray<CachedFeature>,
{
    start: chrono::DateTime<chrono::Utc>,
    // user supplies F defining the features they need
    // The default value of F is defined as 'fallback to string lookups'.
    features: EnumMap<F, CachedFeature>,
    str_features: HashMap<String, CachedFeature>,
}

impl<F> CachedState<F>
where
    F: EnumArray<CachedFeature>,
{
    /// Access the cached string features.
    pub fn str_features(&self) -> &HashMap<String, CachedFeature> {
        &self.str_features
    }
}

pub struct Client<F, C>
where
    F: EnumArray<CachedFeature> + Debug + DeserializeOwned + Serialize,
    C: HttpClient,
{
    api_url: String,
    app_name: String,
    disable_metric_submission: bool,
    enable_str_features: bool,
    instance_id: String,
    interval: u64,
    polling: AtomicBool,
    // Permits making extension calls to the Unleash API not yet modelled in the Rust SDK.
    pub http: HTTP<C>,
    // known strategies: strategy_name : memoiser
    strategies: Mutex<HashMap<String, strategy::Strategy>>,
    // memoised state: feature_name: [callback, callback, ...]
    cached_state: ArcSwapOption<CachedState<F>>,
}

trait Enabled<F>
where
    F: EnumArray<CachedFeature>,
{
    fn is_enabled(&self, feature_enum: F, context: Option<&Context>, default: bool) -> bool;
    fn is_enabled_str(
        &self,
        feature_name: &str,
        context: Option<&Context>,
        default: bool,
        cached_features: &ArcSwapOption<CachedState<F>>,
    ) -> bool;
}

impl<F> Enabled<F> for &Arc<CachedState<F>>
where
    F: EnumArray<CachedFeature> + Clone + Debug + DeserializeOwned + Serialize,
{
    fn is_enabled(&self, feature_enum: F, context: Option<&Context>, default: bool) -> bool {
        trace!(
            "is_enabled: feature {:?} default {}, context {:?}",
            feature_enum,
            default,
            context
        );
        let feature = &self.features[feature_enum.clone()];
        let default_context = &Default::default();
        let context = context.unwrap_or(default_context);

        match (|| {
            if feature.strategies.is_empty() && feature.known && !feature.feature_disabled {
                trace!(
                    "is_enabled: feature {:?} has no strategies: enabling",
                    feature_enum
                );
                return true;
            }
            for memo in feature.strategies.iter() {
                if memo(context) {
                    debug!(
                        "is_enabled: feature {:?} enabled by memo {:p}, context {:?}",
                        feature_enum, memo, context
                    );
                    return true;
                } else {
                    // Traces once per strategy (memo)
                    trace!(
                        "is_enabled: feature {:?} not enabled by memo {:p}, context {:?}",
                        feature_enum,
                        memo,
                        context
                    );
                }
            }
            if !feature.known {
                debug!(
                    "is_enabled: Unknown feature {:?}, using default {}",
                    feature_enum, default
                );
                default
            } else {
                // known, non-empty, missed all strategies: disabled
                debug!(
                    "is_enabled: feature {:?} failed all strategies, disabling",
                    feature_enum
                );
                false
            }
        })() {
            true => {
                feature.enabled.fetch_add(1, Ordering::Relaxed);
                true
            }
            false => {
                feature.disabled.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    fn is_enabled_str(
        &self,
        feature_name: &str,
        context: Option<&Context>,
        default: bool,
        cached_features: &ArcSwapOption<CachedState<F>>,
    ) -> bool {
        if let Some(feature) = &self.str_features.get(feature_name) {
            let default_context: Context = Default::default();
            let context = context.unwrap_or(&default_context);
            if feature.strategies.is_empty() && feature.known && !feature.feature_disabled {
                trace!(
                    "is_enabled: feature {} has no strategies: enabling",
                    feature_name
                );
                feature.enabled.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            for memo in feature.strategies.iter() {
                if memo(context) {
                    debug!(
                        "is_enabled: feature {} enabled by memo {:p}, context {:?}",
                        feature_name, memo, context
                    );
                    feature.enabled.fetch_add(1, Ordering::Relaxed);
                    return true;
                } else {
                    // Traces once per strategy (memo)
                    trace!(
                        "is_enabled: feature {} not enabled by memo {:p}, context {:?}",
                        feature_name,
                        memo,
                        context
                    );
                }
            }
            if !feature.known {
                trace!(
                    "is_enabled: Unknown feature {}, using default {}",
                    feature_name,
                    default
                );
                if default {
                    feature.enabled.fetch_add(1, Ordering::Relaxed);
                } else {
                    feature.disabled.fetch_add(1, Ordering::Relaxed);
                }
                default
            } else {
                false
            }
        } else {
            debug!(
                "is_enabled: Unknown feature {}, using default {}",
                feature_name, default
            );
            // Insert a compiled feature to track metrics.
            cached_features.rcu(|cached_state: &Option<Arc<CachedState<F>>>| {
                // Did someone swap None in ?
                if let Some(cached_state) = cached_state {
                    let cached_state = cached_state.clone();
                    if let Some(feature) = cached_state.str_features.get(feature_name) {
                        // raced with *either* a poll_for_updates() that
                        // added the feature in the API server or another
                        // thread adding this same metric memoisation;
                        // record against metrics here, but still return
                        // default as consistent enough.
                        if default {
                            feature.enabled.fetch_add(1, Ordering::Relaxed);
                        } else {
                            feature.disabled.fetch_add(1, Ordering::Relaxed);
                        }
                        Some(cached_state)
                    } else {
                        // still not present; add it
                        // Build up a new cached state
                        let mut new_state = CachedState {
                            start: cached_state.start,
                            features: EnumMap::default(),
                            str_features: HashMap::new(),
                        };
                        fn cloned_feature(feature: &CachedFeature) -> CachedFeature {
                            CachedFeature {
                                disabled: AtomicU64::new(feature.disabled.load(Ordering::Relaxed)),
                                enabled: AtomicU64::new(feature.enabled.load(Ordering::Relaxed)),
                                disabled_variant_count: AtomicU64::new(
                                    feature.disabled_variant_count.load(Ordering::Relaxed),
                                ),
                                known: feature.known,
                                feature_disabled: feature.feature_disabled,
                                strategies: feature.strategies.clone(),
                                variants: feature.variants.clone(),
                            }
                        }
                        for (key, feature) in &cached_state.features {
                            new_state.features[key] = cloned_feature(feature);
                        }
                        for (name, feature) in &cached_state.str_features {
                            new_state
                                .str_features
                                .insert(name.clone(), cloned_feature(feature));
                        }
                        let stub_feature = CachedFeature {
                            disabled: AtomicU64::new(if default { 0 } else { 1 }),
                            enabled: AtomicU64::new(if default { 1 } else { 0 }),
                            disabled_variant_count: AtomicU64::new(0),
                            known: false,
                            feature_disabled: false,
                            strategies: vec![],
                            variants: vec![],
                        };
                        new_state
                            .str_features
                            .insert(feature_name.into(), stub_feature);
                        Some(Arc::new(new_state))
                    }
                } else {
                    None
                }
            });
            default
        }
    }
}

impl<F, C> Client<F, C>
where
    F: EnumArray<CachedFeature> + Clone + Debug + DeserializeOwned + Serialize,
    C: HttpClient + Default,
{
    /// The cached state can be accessed. It may be uninitialised, and
    /// represents a point in time snapshot: subsequent calls may have wound the
    /// metrics back, entirely lost string features etc.
    pub fn cached_state(&self) -> arc_swap::Guard<Option<Arc<CachedState<F>>>> {
        let cache = self.cached_state.load();
        if cache.is_none() {
            // No API state loaded
            trace!("is_enabled: No API state");
        }
        cache
    }

    /// Determine what variant (if any) of the feature the given context is
    /// selected for. This is a consistent selection within a feature only
    /// - across different features with identical variant definitions,
    ///   different variant selection will take place.
    ///
    /// The key used to hash is the first of the username, sessionid, the host
    /// address, or a random string per call to get_variant.
    pub fn get_variant(&self, feature_enum: F, context: &Context) -> Variant {
        trace!(
            "get_variant: feature {:?} context {:?}",
            feature_enum,
            context
        );
        let cache = self.cached_state();
        let cache = match cache.as_ref() {
            None => {
                trace!("get_variant: feature {:?} no cached state", feature_enum);
                return Variant::disabled();
            }
            Some(cache) => cache,
        };
        let enabled = cache.is_enabled(feature_enum.clone(), Some(context), false);
        let feature = &cache.features[feature_enum.clone()];
        if !enabled {
            feature
                .disabled_variant_count
                .fetch_add(1, Ordering::Relaxed);
            return Variant::disabled();
        }
        let str_f = EnumToString(&feature_enum);
        self._get_variant(feature, str_f, context)
    }

    /// Determine what variant (if any) of the feature the given context is
    /// selected for. This is a consistent selection within a feature only
    /// - across different features with identical variant definitions,
    ///   different variant selection will take place.
    ///
    /// The key used to hash is the first of the username, sessionid, the host
    /// address, or a random string per call to get_variant.
    pub fn get_variant_str(&self, feature_name: &str, context: &Context) -> Variant {
        trace!(
            "get_variant_Str: feature {} context {:?}",
            feature_name,
            context
        );
        assert!(
            self.enable_str_features,
            "String feature lookup not enabled"
        );
        let cache = self.cached_state();
        let cache = match cache.as_ref() {
            None => {
                trace!("get_variant_str: feature {} no cached state", feature_name);
                return Variant::disabled();
            }
            Some(cache) => cache,
        };
        let enabled = cache.is_enabled_str(feature_name, Some(context), false, &self.cached_state);
        let feature = &cache.str_features.get(feature_name);
        if !enabled {
            // Count the disabled variant on the newly created, previously missing feature.
            match feature {
                Some(f) => {
                    f.disabled_variant_count.fetch_add(1, Ordering::Relaxed);
                }
                None => {
                    if let Some(fresh_cache) = self.cached_state().as_ref() {
                        let _ = &fresh_cache
                            .str_features
                            .get(feature_name)
                            .map(|f| f.disabled_variant_count.fetch_add(1, Ordering::Relaxed));
                    }
                }
            }
            return Variant::disabled();
        }
        match feature {
            None => {
                trace!(
                    "get_variant_str: feature {} enabled but not in cache",
                    feature_name
                );
                Variant::disabled()
            }
            Some(feature) => self._get_variant(feature, feature_name, context),
        }
    }

    fn _get_variant<N: Debug + Display>(
        &self,
        feature: &CachedFeature,
        feature_name: N,
        context: &Context,
    ) -> Variant {
        if feature.variants.is_empty() {
            trace!("get_variant: feature {:?} no variants", feature_name);
            feature
                .disabled_variant_count
                .fetch_add(1, Ordering::Relaxed);
            return Variant::disabled();
        }
        let group = format!("{}", feature_name);
        let mut remote_address: Option<String> = None;
        let identifier = context
            .user_id
            .as_ref()
            .or(context.session_id.as_ref())
            .or_else(|| {
                context.remote_address.as_ref().and_then({
                    |addr| {
                        remote_address = Some(format!("{:?}", addr));
                        remote_address.as_ref()
                    }
                })
            });
        if identifier.is_none() {
            trace!(
                "get_variant: feature {:?} context has no identifiers, selecting randomly",
                feature_name
            );
            let mut rng = rand::thread_rng();
            let picked = rng.gen_range(0..feature.variants.len());
            feature.variants[picked]
                .count
                .fetch_add(1, Ordering::Relaxed);
            return (&feature.variants[picked]).into();
        }
        let identifier = identifier.unwrap();
        let total_weight = feature.variants.iter().map(|v| v.weight as u32).sum();
        strategy::normalised_hash(&group, identifier, total_weight)
            .map(|selected_weight| {
                let mut counter: u32 = 0;
                for variant in feature.variants.iter().as_ref() {
                    counter += variant.weight as u32;
                    if counter > selected_weight {
                        variant.count.fetch_add(1, Ordering::Relaxed);
                        return variant.into();
                    }
                }

                feature
                    .disabled_variant_count
                    .fetch_add(1, Ordering::Relaxed);
                Variant::disabled()
            })
            .unwrap_or_else(|_| {
                feature
                    .disabled_variant_count
                    .fetch_add(1, Ordering::Relaxed);

                Variant::disabled()
            })
    }

    pub fn is_enabled(&self, feature_enum: F, context: Option<&Context>, default: bool) -> bool {
        trace!(
            "is_enabled: feature {:?} default {}, context {:?}",
            feature_enum,
            default,
            context
        );
        let cache = self.cached_state();
        let cache = match cache.as_ref() {
            None => {
                trace!("is_enabled: feature {:?} no cached state", feature_enum);
                return false;
            }
            Some(cache) => cache,
        };
        cache.is_enabled(feature_enum, context, default)
    }

    pub fn is_enabled_str(
        &self,
        feature_name: &str,
        context: Option<&Context>,
        default: bool,
    ) -> bool {
        trace!(
            "is_enabled: feature_str {:?} default {}, context {:?}",
            feature_name,
            default,
            context
        );
        assert!(
            self.enable_str_features,
            "String feature lookup not enabled"
        );
        let cache = self.cached_state();
        let cache = match cache.as_ref() {
            None => return false,
            Some(cache) => cache,
        };
        cache.is_enabled_str(feature_name, context, default, &self.cached_state)
    }

    /// Memoize new features into the cached state
    ///
    /// Interior mutability is used, via the arc-swap crate.
    ///
    /// Note that this is primarily public to facilitate benchmarking;
    /// poll_for_updates is the usual way in which memoize will be called.
    pub fn memoize(
        &self,
        features: Vec<Feature>,
    ) -> Result<Option<Metrics>, Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now();
        trace!("memoize: start with {} features", features.len());
        let source_strategies = self.strategies.lock().unwrap();
        let mut unenumerated_features: HashMap<String, CachedFeature> = HashMap::new();
        let mut cached_features: EnumMap<F, CachedFeature> = EnumMap::default();
        // HashMap<String, Vec<Box<strategy::Evaluate>>> = HashMap::new();
        for feature in features {
            let cached_feature = {
                if !feature.enabled {
                    // no strategies == return false per the unleash example code;
                    let strategies = vec![];
                    CachedFeature {
                        strategies,
                        disabled: AtomicU64::new(0),
                        enabled: AtomicU64::new(0),
                        disabled_variant_count: AtomicU64::new(0),
                        known: true,
                        feature_disabled: true,
                        variants: vec![],
                    }
                } else {
                    // TODO add variant support
                    let mut strategies = vec![];
                    for api_strategy in feature.strategies {
                        if let Some(code_strategy) = source_strategies.get(&api_strategy.name) {
                            strategies.push(strategy::constrain(
                                api_strategy.constraints,
                                code_strategy,
                                api_strategy.parameters,
                            ));
                        }
                        // Graceful degradation: ignore this unknown strategy.
                        // TODO: add a logging layer and log it.
                    }
                    // Only include variants where the weight is greater than zero to save filtering at query time
                    let variants = feature
                        .variants
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|v| v.weight > 0)
                        .map(Into::into)
                        .collect();
                    CachedFeature {
                        strategies,
                        disabled: AtomicU64::new(0),
                        enabled: AtomicU64::new(0),
                        disabled_variant_count: AtomicU64::new(0),
                        known: true,
                        feature_disabled: false,
                        variants,
                    }
                }
            };
            if let Ok(feature_enum) = serde_plain::from_str::<F>(feature.name.as_str()) {
                cached_features[feature_enum] = cached_feature;
            } else {
                unenumerated_features.insert(feature.name.clone(), cached_feature);
            }
        }
        let new_cache = CachedState {
            start: now,
            features: cached_features,
            str_features: unenumerated_features,
        };
        // Now we have the new cache compiled, swap it in.
        let old = self.cached_state.swap(Some(Arc::new(new_cache)));
        trace!("memoize: swapped memoized state in");
        if let Some(old) = old {
            // send metrics here
            let mut bucket = MetricsBucket {
                start: old.start,
                stop: now,
                toggles: HashMap::new(),
            };
            for (key, feature) in &old.features {
                bucket.toggles.insert(
                    // Is this unwrap safe? Not sure.
                    serde_plain::to_string(&key).unwrap(),
                    [
                        ("yes".into(), feature.enabled.load(Ordering::Relaxed)),
                        ("no".into(), feature.disabled.load(Ordering::Relaxed)),
                    ]
                    .iter()
                    .cloned()
                    .collect(),
                );
            }
            for (name, feature) in &old.str_features {
                bucket.toggles.insert(
                    name.clone(),
                    [
                        ("yes".into(), feature.enabled.load(Ordering::Relaxed)),
                        ("no".into(), feature.disabled.load(Ordering::Relaxed)),
                    ]
                    .iter()
                    .cloned()
                    .collect(),
                );
            }
            let metrics = Metrics {
                app_name: self.app_name.clone(),
                instance_id: self.instance_id.clone(),
                bucket,
            };
            Ok(Some(metrics))
        } else {
            Ok(None)
        }
    }

    /// Query the API endpoint for features and push metrics
    ///
    /// Immediately and then every self.interval milliseconds the API server is
    /// queryed for features and the previous cycles metrics are uploaded.
    ///
    /// May be dropped, or will terminate at the next polling cycle after
    /// stop_poll is called().
    pub async fn poll_for_updates(&self) {
        // TODO: add an event / pipe to permit immediate exit.
        let endpoint = Features::endpoint(&self.api_url);
        let metrics_endpoint = Metrics::endpoint(&self.api_url);
        self.polling.store(true, Ordering::Relaxed);
        loop {
            debug!("poll: retrieving features");
            let res = self.http.get_json(&endpoint).await;
            if let Ok(res) = res {
                let features: Features = res;
                match self.memoize(features.features) {
                    Ok(None) => {}
                    Ok(Some(metrics)) => {
                        if !self.disable_metric_submission {
                            let mut metrics_uploaded = false;
                            let res = self.http.post_json(&metrics_endpoint, metrics).await;
                            if let Ok(successful) = res {
                                if successful {
                                    metrics_uploaded = true;
                                    debug!("poll: uploaded feature metrics")
                                }
                            }
                            if !metrics_uploaded {
                                warn!("poll: error uploading feature metrics");
                            }
                        }
                    }
                    Err(_) => {
                        warn!("poll: failed to memoize features");
                    }
                }
            } else {
                warn!("poll: failed to retrieve features");
            }

            let duration = Duration::from_millis(self.interval);
            debug!("poll: waiting {:?}", duration);
            Delay::new(duration).await;

            if !self.polling.load(Ordering::Relaxed) {
                return;
            }
        }
    }

    /// Register this client with the API endpoint.
    pub async fn register(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let registration = Registration {
            app_name: self.app_name.clone(),
            instance_id: self.instance_id.clone(),
            interval: self.interval,
            strategies: self
                .strategies
                .lock()
                .unwrap()
                .keys()
                .map(|s| s.to_owned())
                .collect(),
            ..Default::default()
        };
        let success = self
            .http
            .post_json(&Registration::endpoint(&self.api_url), &registration)
            .await
            .map_err(|err| anyhow::anyhow!(err))?;
        if !success {
            return Err(anyhow::anyhow!("Failed to register with unleash API server").into());
        }
        Ok(())
    }

    /// stop the poll_for_updates() function.
    ///
    /// If poll is not running, will wait-loop until poll_for_updates is
    /// running, then signal it to stop, then return. Will wait for ever if
    /// poll_for_updates never starts running.
    pub async fn stop_poll(&self) {
        loop {
            match self
                .polling
                .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    return;
                }
                Err(_) => {
                    Delay::new(Duration::from_millis(50)).await;
                }
            }
        }
    }
}

// DisplayForEnum

/// Adapts an Enum to have Display for _get_variant so we can give consistent
/// results between get_variant and get_variant_str on the same feature.
struct EnumToString<T>(T)
where
    T: Debug;

impl<T> Debug for EnumToString<T>
where
    T: Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        self.0.fmt(formatter)
    }
}

impl<T> Display for EnumToString<T>
where
    T: Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::HashMap;
    use std::collections::hash_set::HashSet;
    use std::default::Default;
    use std::hash::BuildHasher;

    use enum_map::Enum;
    use maplit::hashmap;
    use serde::{Deserialize, Serialize};

    use super::{ClientBuilder, Variant};
    use crate::api::{self, Feature, Features, Strategy};
    use crate::context::{Context, IPAddress};
    use crate::strategy;

    cfg_if::cfg_if! {
        if #[cfg(feature = "surf")] {
            use surf::Client as HttpClient;
        } else if #[cfg(feature = "reqwest")] {
            use reqwest::Client as HttpClient;
        } else {
            compile_error!("Cannot run test suite without a client enabled");
        }
    }

    fn features() -> Features {
        Features {
            version: 1,
            features: vec![
                Feature {
                    description: "default".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "default".into(),
                    strategies: vec![Strategy {
                        name: "default".into(),
                        ..Default::default()
                    }],
                },
                Feature {
                    description: "userWithId".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "userWithId".into(),
                    strategies: vec![Strategy {
                        name: "userWithId".into(),
                        parameters: Some(hashmap!["userIds".into()=>"present".into()]),
                        ..Default::default()
                    }],
                },
                Feature {
                    description: "userWithId+default".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "userWithId+default".into(),
                    strategies: vec![
                        Strategy {
                            name: "userWithId".into(),
                            parameters: Some(hashmap!["userIds".into()=>"present".into()]),
                            ..Default::default()
                        },
                        Strategy {
                            name: "default".into(),
                            ..Default::default()
                        },
                    ],
                },
                Feature {
                    description: "disabled".into(),
                    enabled: false,
                    created_at: None,
                    variants: None,
                    name: "disabled".into(),
                    strategies: vec![Strategy {
                        name: "default".into(),
                        ..Default::default()
                    }],
                },
                Feature {
                    description: "nostrategies".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "nostrategies".into(),
                    strategies: vec![],
                },
            ],
        }
    }

    #[test]
    fn test_memoization_enum() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let f = features();
        // with an enum
        #[allow(non_camel_case_types)]
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum UserFeatures {
            unknown,
            default,
            userWithId,
            #[serde(rename = "userWithId+default")]
            userWithId_Default,
            disabled,
            nostrategies,
        }
        let c = ClientBuilder::default()
            .into_client::<UserFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        c.memoize(f.features).unwrap();
        let present: Context = Context {
            user_id: Some("present".into()),
            ..Default::default()
        };
        let missing: Context = Context {
            user_id: Some("missing".into()),
            ..Default::default()
        };
        // features unknown on the server should honour the default
        assert!(!c.is_enabled(UserFeatures::unknown, None, false));
        assert!(c.is_enabled(UserFeatures::unknown, None, true));
        // default should be enabled, no context needed
        assert!(c.is_enabled(UserFeatures::default, None, false));
        // user present should be present on userWithId
        assert!(c.is_enabled(UserFeatures::userWithId, Some(&present), false));
        // user missing should not
        assert!(!c.is_enabled(UserFeatures::userWithId, Some(&missing), false));
        // user missing should be present on userWithId+default
        assert!(c.is_enabled(UserFeatures::userWithId_Default, Some(&missing), false));
        // disabled should be disabled
        assert!(!c.is_enabled(UserFeatures::disabled, None, true));
        // no strategies should result in enabled features.
        assert!(c.is_enabled(UserFeatures::nostrategies, None, false));
    }

    #[test]
    fn test_memoization_strs() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let f = features();
        // And with plain old strings
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum NoFeatures {}
        let c = ClientBuilder::default()
            .enable_string_features()
            .into_client::<NoFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        c.memoize(f.features).unwrap();
        let present: Context = Context {
            user_id: Some("present".into()),
            ..Default::default()
        };
        let missing: Context = Context {
            user_id: Some("missing".into()),
            ..Default::default()
        };
        // features unknown on the server should honour the default
        assert!(!c.is_enabled_str("unknown", None, false));
        assert!(c.is_enabled_str("unknown", None, true));
        // default should be enabled, no context needed
        assert!(c.is_enabled_str("default", None, false));
        // user present should be present on userWithId
        assert!(c.is_enabled_str("userWithId", Some(&present), false));
        // user missing should not
        assert!(!c.is_enabled_str("userWithId", Some(&missing), false));
        // user missing should be present on userWithId+default
        assert!(c.is_enabled_str("userWithId+default", Some(&missing), false));
        // disabled should be disabled
        assert!(!c.is_enabled_str("disabled", None, true));
        // no strategies should result in enabled features.
        assert!(c.is_enabled_str("nostrategies", None, false));
    }

    fn _reversed_uids<S: BuildHasher>(
        parameters: Option<HashMap<String, String, S>>,
    ) -> strategy::Evaluate {
        let mut uids: HashSet<String> = HashSet::new();
        if let Some(parameters) = parameters {
            if let Some(uids_list) = parameters.get("userIds") {
                for uid in uids_list.split(',') {
                    uids.insert(uid.chars().rev().collect());
                }
            }
        }
        Box::new(move |context: &Context| -> bool {
            context
                .user_id
                .as_ref()
                .map(|uid| uids.contains(uid))
                .unwrap_or(false)
        })
    }

    #[test]
    fn test_custom_strategy() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        #[allow(non_camel_case_types)]
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum UserFeatures {
            default,
            reversed,
        }
        let client = ClientBuilder::default()
            .strategy("reversed", Box::new(&_reversed_uids))
            .into_client::<UserFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        let f = Features {
            version: 1,
            features: vec![
                Feature {
                    description: "default".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "default".into(),
                    strategies: vec![Strategy {
                        name: "default".into(),
                        ..Default::default()
                    }],
                },
                Feature {
                    description: "reversed".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "reversed".into(),
                    strategies: vec![Strategy {
                        name: "reversed".into(),
                        parameters: Some(hashmap!["userIds".into()=>"abc".into()]),
                        ..Default::default()
                    }],
                },
            ],
        };
        client.memoize(f.features).unwrap();
        let present: Context = Context {
            user_id: Some("cba".into()),
            ..Default::default()
        };
        let missing: Context = Context {
            user_id: Some("abc".into()),
            ..Default::default()
        };
        // user cba should be present on reversed
        assert!(client.is_enabled(UserFeatures::reversed, Some(&present), false));
        // user abc should not
        assert!(!client.is_enabled(UserFeatures::reversed, Some(&missing), false));
        // adding custom strategies shouldn't disable built-in ones
        // default should be enabled, no context needed
        assert!(client.is_enabled(UserFeatures::default, None, false));
    }

    fn variant_features() -> Features {
        Features {
            version: 1,
            features: vec![
                Feature {
                    description: "disabled".into(),
                    enabled: false,
                    created_at: None,
                    variants: None,
                    name: "disabled".into(),
                    strategies: vec![],
                },
                Feature {
                    description: "novariants".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "novariants".into(),
                    strategies: vec![Strategy {
                        name: "default".into(),
                        ..Default::default()
                    }],
                },
                Feature {
                    description: "one".into(),
                    enabled: true,
                    created_at: None,
                    variants: Some(vec![api::Variant {
                        name: "variantone".into(),
                        weight: 100,
                        payload: Some(hashmap![
                            "type".into() => "string".into(),
                            "value".into() => "val1".into()]),
                        overrides: None,
                    }]),
                    name: "one".into(),
                    strategies: vec![],
                },
                Feature {
                    description: "two".into(),
                    enabled: true,
                    created_at: None,
                    variants: Some(vec![
                        api::Variant {
                            name: "variantone".into(),
                            weight: 50,
                            payload: Some(hashmap![
                            "type".into() => "string".into(),
                            "value".into() => "val1".into()]),
                            overrides: None,
                        },
                        api::Variant {
                            name: "varianttwo".into(),
                            weight: 50,
                            payload: Some(hashmap![
                            "type".into() => "string".into(),
                            "value".into() => "val2".into()]),
                            overrides: None,
                        },
                    ]),
                    name: "two".into(),
                    strategies: vec![],
                },
                Feature {
                    description: "nostrategies".into(),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "nostrategies".into(),
                    strategies: vec![],
                },
            ],
        }
    }

    #[test]
    fn variants_enum() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let f = variant_features();
        // with an enum
        #[allow(non_camel_case_types)]
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum UserFeatures {
            disabled,
            novariants,
            one,
            two,
        }
        let c = ClientBuilder::default()
            .into_client::<UserFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        c.memoize(f.features).unwrap();

        // disabled should be disabled
        let variant = Variant::disabled();
        assert_eq!(
            variant,
            c.get_variant(UserFeatures::disabled, &Context::default())
        );

        // enabled no variants should get the disabled variant
        let variant = Variant::disabled();
        assert_eq!(
            variant,
            c.get_variant(UserFeatures::novariants, &Context::default())
        );

        // One variant
        let variant = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
        };
        assert_eq!(
            variant,
            c.get_variant(UserFeatures::one, &Context::default())
        );

        // Two variants
        let uid1: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        let session1: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        let host1: Context = Context {
            remote_address: Some(IPAddress("10.10.10.10".parse().unwrap())),
            ..Default::default()
        };
        let variant1 = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
        };
        let variant2 = Variant {
            name: "varianttwo".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val2".into()
            ],
            enabled: true,
        };
        assert_eq!(variant2, c.get_variant(UserFeatures::two, &uid1));
        assert_eq!(variant2, c.get_variant(UserFeatures::two, &session1));
        assert_eq!(variant1, c.get_variant(UserFeatures::two, &host1));
    }

    #[test]
    fn variants_str() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let f = variant_features();
        // without the enum API
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum NoFeatures {}
        let c = ClientBuilder::default()
            .enable_string_features()
            .into_client::<NoFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        c.memoize(f.features).unwrap();

        // disabled should be disabled
        let variant = Variant::disabled();
        assert_eq!(variant, c.get_variant_str("disabled", &Context::default()));

        // enabled no variants should get the disabled variant
        let variant = Variant::disabled();
        assert_eq!(
            variant,
            c.get_variant_str("novariants", &Context::default())
        );

        // One variant
        let variant = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
        };
        assert_eq!(variant, c.get_variant_str("one", &Context::default()));

        // Two variants
        let uid1: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        let session1: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        let host1: Context = Context {
            remote_address: Some(IPAddress("10.10.10.10".parse().unwrap())),
            ..Default::default()
        };
        let variant1 = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
        };
        let variant2 = Variant {
            name: "varianttwo".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val2".into()
            ],
            enabled: true,
        };
        assert_eq!(variant2, c.get_variant_str("two", &uid1));
        assert_eq!(variant2, c.get_variant_str("two", &session1));
        assert_eq!(variant1, c.get_variant_str("two", &host1));
    }

    #[test]
    fn variant_metrics() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let f = variant_features();
        // with an enum
        #[allow(non_camel_case_types)]
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum UserFeatures {
            disabled,
            novariants,
            one,
            two,
        }
        let c = ClientBuilder::default()
            .into_client::<UserFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        c.memoize(f.features).unwrap();

        let disabled_variant_count = |feature_name| -> u64 {
            *c.cached_state().clone().expect("No cached state").features[feature_name]
                .variant_metrics()
                .get("disabled")
                .unwrap()
        };

        c.get_variant(UserFeatures::disabled, &Context::default());
        assert_eq!(disabled_variant_count(UserFeatures::disabled), 1);

        c.get_variant(UserFeatures::novariants, &Context::default());
        assert_eq!(disabled_variant_count(UserFeatures::novariants), 1);

        let session1: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };

        let host1: Context = Context {
            remote_address: Some(IPAddress("10.10.10.10".parse().unwrap())),
            ..Default::default()
        };
        c.get_variant(UserFeatures::two, &session1);
        c.get_variant(UserFeatures::two, &host1);

        let variant_count = |feature_name, variant_name| -> u64 {
            *c.cached_state().clone().expect("No cached state").features[feature_name]
                .variant_metrics()
                .get(variant_name)
                .unwrap()
        };

        assert_eq!(variant_count(UserFeatures::two, "variantone"), 1);
        assert_eq!(variant_count(UserFeatures::two, "varianttwo"), 1);
    }

    #[test]
    fn variant_metrics_str() {
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let f = variant_features();
        // with an enum
        #[allow(non_camel_case_types)]
        #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
        enum NoFeatures {}
        let c = ClientBuilder::default()
            .enable_string_features()
            .into_client::<NoFeatures, HttpClient>("http://127.0.0.1:1234/", "foo", "test", None)
            .unwrap();

        c.memoize(f.features).unwrap();

        let disabled_variant_count = |feature_name| -> u64 {
            *c.cached_state()
                .clone()
                .expect("No cached state")
                .str_features
                .get(feature_name)
                .expect("No feature named {feature_name}")
                .variant_metrics()
                .get("disabled")
                .unwrap()
        };

        c.get_variant_str("disabled", &Context::default());
        assert_eq!(disabled_variant_count("disabled"), 1);

        c.get_variant_str("novariants", &Context::default());
        assert_eq!(disabled_variant_count("novariants"), 1);

        let session1: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };

        let host1: Context = Context {
            remote_address: Some(IPAddress("10.10.10.10".parse().unwrap())),
            ..Default::default()
        };
        c.get_variant_str("two", &session1);
        c.get_variant_str("two", &host1);

        let variant_count = |feature_name, variant_name| -> u64 {
            *c.cached_state()
                .clone()
                .expect("No cached state")
                .str_features
                .get(feature_name)
                .expect("No feature named {feature_name}")
                .variant_metrics()
                .get(variant_name)
                .unwrap()
        };

        assert_eq!(variant_count("two", "variantone"), 1);
        assert_eq!(variant_count("two", "varianttwo"), 1);

        // Metrics should also be tracked for features that don't exist
        c.get_variant_str("nonexistant-feature", &Context::default());
        assert_eq!(variant_count("nonexistant-feature", "disabled"), 1);

        c.get_variant_str("nonexistant-feature", &Context::default());
        assert_eq!(variant_count("nonexistant-feature", "disabled"), 2);

        // Calling is_enabled_str shouldn't increment disabled variant counts
        c.is_enabled_str("bogus-feature", None, false);
        assert_eq!(variant_count("bogus-feature", "disabled"), 0);
    }
}
