// Copyright 2020 Cognite AS
//! The primary interface for users of the library.
use std::collections::hash_map::HashMap;
use std::default::Default;
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use chrono::Utc;
use enum_map::{Enum, EnumMap};
use futures_timer::Delay;
use log::{debug, trace, warn};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::api::{Feature, Features, Metrics, MetricsBucket, Registration};
use crate::context::Context;
use crate::http::HTTP;
use crate::strategy;

pub struct ClientBuilder {
    disable_metric_submission: bool,
    enable_str_features: bool,
    interval: u64,
    strategies: HashMap<String, strategy::Strategy>,
}

impl ClientBuilder {
    pub fn into_client<C, F>(
        self,
        api_url: &str,
        app_name: &str,
        instance_id: &str,
        authorization: Option<String>,
    ) -> Result<Client<C, F>, http_client::Error>
    where
        C: http_client::HttpClient + Default,
        F: Enum<CachedFeature> + Debug + DeserializeOwned + Serialize,
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
}

pub struct CachedState<F>
where
    F: Enum<CachedFeature>,
{
    start: chrono::DateTime<chrono::Utc>,
    // user supplies F defining the features they need
    // The default value of F is defined as 'fallback to string lookups'.
    features: EnumMap<F, CachedFeature>,
    str_features: HashMap<String, CachedFeature>,
}

impl<F> CachedState<F>
where
    F: Enum<CachedFeature>,
{
    /// Access the cached string features.
    pub fn str_features(&self) -> &HashMap<String, CachedFeature> {
        &self.str_features
    }
}

pub struct Client<C, F>
where
    C: http_client::HttpClient,
    F: Enum<CachedFeature> + Debug + DeserializeOwned + Serialize,
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

impl<C, F> Client<C, F>
where
    C: http_client::HttpClient + std::default::Default,
    F: Enum<CachedFeature> + Clone + Debug + DeserializeOwned + Serialize,
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

    pub fn is_enabled(&self, feature_enum: F, context: Option<&Context>, default: bool) -> bool {
        trace!(
            "is_enabled: feature {:?} default {}, context {:?}",
            feature_enum,
            default,
            context
        );
        let cache = self.cached_state();
        let cache = match cache.as_ref() {
            None => return false,
            Some(cache) => cache,
        };
        let feature = &cache.features[feature_enum.clone()];
        let default_context: Context = Default::default();
        let context = context.unwrap_or(&default_context);
        if feature.strategies.is_empty() && feature.known && !feature.feature_disabled {
            trace!(
                "is_enabled: feature {:?} has no strategies: enabling",
                feature_enum
            );
            feature.enabled.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        for memo in feature.strategies.iter() {
            if memo(context) {
                debug!(
                    "is_enabled: feature {:?} enabled by memo {:p}, context {:?}",
                    feature_enum, memo, context
                );
                feature.enabled.fetch_add(1, Ordering::Relaxed);
                return true;
            } else {
                feature.disabled.fetch_add(1, Ordering::Relaxed);
                trace!(
                    "is_enabled: feature {:?} not enabled by memo {:p}, context {:?}",
                    feature_enum,
                    memo,
                    context
                );
            }
        }
        if !feature.known {
            trace!(
                "is_enabled: Unknown feature {:?}, using default {}",
                feature_enum,
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
        if let Some(feature) = cache.str_features.get(feature_name) {
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
                    feature.disabled.fetch_add(1, Ordering::Relaxed);
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
            trace!(
                "is_enabled: Unknown feature {}, using default {}",
                feature_name,
                default
            );
            // Insert a compiled feature to track metrics.
            self.cached_state
                .rcu(|cached_state: &Option<Arc<CachedState<F>>>| {
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
                                features: EnumMap::new(),
                                str_features: HashMap::new(),
                            };
                            fn cloned_feature(feature: &CachedFeature) -> CachedFeature {
                                CachedFeature {
                                    disabled: AtomicU64::new(
                                        feature.disabled.load(Ordering::Relaxed),
                                    ),
                                    enabled: AtomicU64::new(
                                        feature.enabled.load(Ordering::Relaxed),
                                    ),
                                    known: feature.known,
                                    feature_disabled: feature.feature_disabled,
                                    strategies: feature.strategies.clone(),
                                }
                            };
                            for (key, feature) in &cached_state.features {
                                new_state.features[key] = cloned_feature(&feature);
                            }
                            for (name, feature) in &cached_state.str_features {
                                new_state
                                    .str_features
                                    .insert(name.clone(), cloned_feature(&feature));
                            }
                            let stub_feature = CachedFeature {
                                disabled: AtomicU64::new(if default { 0 } else { 1 }),
                                enabled: AtomicU64::new(if default { 1 } else { 0 }),
                                known: false,
                                feature_disabled: false,
                                strategies: vec![],
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

    /// Memoize new features into the cached state
    ///
    /// Interior mutability is used, via the arc-swap crate.
    ///
    /// Note that this is primarily public to facilitate benchmarking;
    /// poll_for_updates is the usual way in which memoize will be called.
    pub fn memoize(
        &self,
        features: Vec<Feature>,
    ) -> Result<Option<Metrics>, Box<dyn std::error::Error>> {
        let now = Utc::now();
        trace!("memoize: start with {} features", features.len());
        let source_strategies = self.strategies.lock().unwrap();
        let mut unenumerated_features: HashMap<String, CachedFeature> = HashMap::new();
        let mut cached_features: EnumMap<F, CachedFeature> = EnumMap::new();
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
                        known: true,
                        feature_disabled: true,
                    }
                } else {
                    // TODO add variant support
                    let mut strategies = vec![];
                    for api_strategy in feature.strategies {
                        if let Some(code_strategy) = source_strategies.get(&api_strategy.name) {
                            strategies.push(code_strategy(api_strategy.parameters));
                        }
                        // Graceful degradation: ignore this unknown strategy.
                        // TODO: add a logging layer and log it.
                    }
                    CachedFeature {
                        strategies,
                        disabled: AtomicU64::new(0),
                        enabled: AtomicU64::new(0),
                        known: true,
                        feature_disabled: false,
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
            let res = self.http.get(&endpoint).recv_json().await;
            if let Ok(res) = res {
                let features: Features = res;
                match self.memoize(features.features) {
                    Ok(None) => {}
                    Ok(Some(metrics)) => {
                        if !self.disable_metric_submission {
                            let mut metrics_uploaded = false;
                            let req = self.http.post(&metrics_endpoint).body_json(&metrics);
                            if let Ok(req) = req {
                                let res = req.await;
                                if let Ok(res) = res {
                                    if res.status().is_success() {
                                        metrics_uploaded = true;
                                        debug!("poll: uploaded feature metrics")
                                    }
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
        let res = self
            .http
            .post(Registration::endpoint(&self.api_url))
            .body_json(&registration)?
            .await?;
        if !res.status().is_success() {
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

#[cfg(test)]
mod tests {
    use std::collections::hash_map::HashMap;
    use std::collections::hash_set::HashSet;
    use std::hash::BuildHasher;

    use enum_map::Enum;
    use maplit::hashmap;
    use serde::{Deserialize, Serialize};

    use super::ClientBuilder;
    use crate::api::{Feature, Features, Strategy};
    use crate::context::Context;
    use crate::strategy;

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
                        parameters: None,
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
                        },
                        Strategy {
                            name: "default".into(),
                            parameters: None,
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
                        parameters: None,
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
            .into_client::<http_client::native::NativeClient, UserFeatures>(
                "http://127.0.0.1:1234/",
                "foo",
                "test",
                None,
            )
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
        assert_eq!(false, c.is_enabled(UserFeatures::unknown, None, false));
        assert_eq!(true, c.is_enabled(UserFeatures::unknown, None, true));
        // default should be enabled, no context needed
        assert_eq!(true, c.is_enabled(UserFeatures::default, None, false));
        // user present should be present on userWithId
        assert_eq!(
            true,
            c.is_enabled(UserFeatures::userWithId, Some(&present), false)
        );
        // user missing should not
        assert_eq!(
            false,
            c.is_enabled(UserFeatures::userWithId, Some(&missing), false)
        );
        // user missing should be present on userWithId+default
        assert_eq!(
            true,
            c.is_enabled(UserFeatures::userWithId_Default, Some(&missing), false)
        );
        // disabled should be disabled
        assert_eq!(false, c.is_enabled(UserFeatures::disabled, None, true));
        // no strategies should result in enabled features.
        assert_eq!(true, c.is_enabled(UserFeatures::nostrategies, None, false));
    }

    #[test]
    fn test_memoization_strs() {
        let _ = simple_logger::SimpleLogger::new()
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
            .into_client::<http_client::native::NativeClient, NoFeatures>(
                "http://127.0.0.1:1234/",
                "foo",
                "test",
                None,
            )
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
        assert_eq!(false, c.is_enabled_str("unknown", None, false));
        assert_eq!(true, c.is_enabled_str("unknown", None, true));
        // default should be enabled, no context needed
        assert_eq!(true, c.is_enabled_str("default", None, false));
        // user present should be present on userWithId
        assert_eq!(true, c.is_enabled_str("userWithId", Some(&present), false));
        // user missing should not
        assert_eq!(false, c.is_enabled_str("userWithId", Some(&missing), false));
        // user missing should be present on userWithId+default
        assert_eq!(
            true,
            c.is_enabled_str("userWithId+default", Some(&missing), false)
        );
        // disabled should be disabled
        assert_eq!(false, c.is_enabled_str("disabled", None, true));
        // no strategies should result in enabled features.
        assert_eq!(true, c.is_enabled_str("nostrategies", None, false));
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
            .into_client::<http_client::native::NativeClient, UserFeatures>(
                "http://127.0.0.1:1234/",
                "foo",
                "test",
                None,
            )
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
                        parameters: None,
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
        assert_eq!(
            true,
            client.is_enabled(UserFeatures::reversed, Some(&present), false)
        );
        // user abc should not
        assert_eq!(
            false,
            client.is_enabled(UserFeatures::reversed, Some(&missing), false)
        );
        // adding custom strategies shouldn't disable built-in ones
        // default should be enabled, no context needed
        assert_eq!(true, client.is_enabled(UserFeatures::default, None, false));
    }
}
