// Copyright 2020 Cognite AS
//! The primary interface for users of the library.
use std::collections::hash_map::HashMap;
use std::default::Default;
use std::fmt::Debug;
use std::iter::FromIterator;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use chrono::Utc;
use enum_map::EnumArray;
use futures_timer::Delay;
use log::{debug, trace, warn};
use serde::de::DeserializeOwned;
use serde::Serialize;
use unleash_yggdrasil::state::EnrichedContext;
use unleash_yggdrasil::{EngineState, UpdateMessage};
use uuid::Uuid;

use crate::api::{features_endpoint, Metrics, Registration};
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
    pub feature_enabled: bool,
}

impl Variant {
    fn disabled(feature_enabled: bool) -> Self {
        Self {
            name: "disabled".into(),
            feature_enabled,
            ..Default::default()
        }
    }
}

fn build_yggdrasil_context(context: &Context, feature_name: &str) -> EnrichedContext {
    EnrichedContext {
        user_id: context.user_id.clone(),
        session_id: context.session_id.clone(),
        environment: Some(context.environment.clone()),
        app_name: Some(context.app_name.clone()),
        current_time: context.current_time.as_ref().map(|time| time.to_string()),
        remote_address: context
            .remote_address
            .as_ref()
            .map(|remote_addr| remote_addr.0.to_string()),
        properties: Some(context.properties.clone()),
        external_results: None,
        toggle_name: feature_name.to_string(),
        runtime_hostname: None, //explicitly set to None, this is an escape hatch for environments like WASM where this cannot be resolved
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
        F: EnumArray<()> + Debug + DeserializeOwned + Serialize,
        C: HttpClient + Default,
    {
        let connection_id = Uuid::new_v4().to_string();
        Ok(Client {
            api_url: api_url.into(),
            app_name: app_name.into(),
            disable_metric_submission: self.disable_metric_submission,
            instance_id: instance_id.into(),
            connection_id: connection_id.clone(),
            interval: self.interval,
            polling: AtomicBool::new(false),
            http: HTTP::new(
                app_name.into(),
                instance_id.into(),
                connection_id,
                authorization,
            )?,
            cached_state: ArcSwapOption::from(None),
            strategies: Mutex::new(self.strategies),
            _phantom: PhantomData::<F>,
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
        ClientBuilder {
            disable_metric_submission: false,
            enable_str_features: false,
            interval: 15000,
            strategies: Default::default(),
        }
    }
}

pub struct Client<F, C>
where
    F: EnumArray<()> + Debug + DeserializeOwned + Serialize,
    C: HttpClient,
{
    api_url: String,
    app_name: String,
    disable_metric_submission: bool,
    instance_id: String,
    connection_id: String,
    interval: u64,
    polling: AtomicBool,
    // Permits making extension calls to the Unleash API not yet modelled in the Rust SDK.
    pub http: HTTP<C>,
    // known strategies: strategy_name : memoiser
    strategies: Mutex<HashMap<String, strategy::Strategy>>,
    cached_state: ArcSwapOption<EngineState>,
    _phantom: PhantomData<F>,
}

impl<F, C> Client<F, C>
where
    F: EnumArray<()> + Debug + DeserializeOwned + Serialize,
    C: HttpClient + Default,
{
    /// The cached state can be accessed. It may be uninitialised, and
    /// represents a point in time snapshot: subsequent calls may have wound the
    /// metrics back, entirely lost string features etc.
    pub fn cached_state(&self) -> arc_swap::Guard<Option<Arc<EngineState>>> {
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
        let feature_name =
            serde_plain::to_string(&feature_enum).expect("Failed to resolve feature name");

        self.get_variant_str(&feature_name, context)
    }

    /// Determine what variant (if any) of the feature the given context is
    /// selected for. This is a consistent selection within a feature only
    /// - across different features with identical variant definitions,
    ///   different variant selection will take place.
    ///
    /// The key used to hash is the first of the username, sessionid, the host
    /// address, or a random string per call to get_variant.
    pub fn get_variant_str(&self, feature_name: &str, context: &Context) -> Variant {
        let cache = self.cached_state();
        let Some(cache) = cache.as_ref() else {
            return Variant::disabled(false);
        };
        let context = build_yggdrasil_context(context, feature_name);

        let feature_enabled = cache.check_enabled(&context).unwrap_or(false);
        let yggdrasil_variant = cache.check_variant(&context);

        cache.count_toggle(feature_name, feature_enabled);
        cache.count_variant(
            feature_name,
            &yggdrasil_variant
                .as_ref()
                .map(|v| v.name.clone())
                .unwrap_or_else(|| "disabled".into()),
        );

        yggdrasil_variant
            .map(|variant_def| {
                let payload = if let Some(original_payload) = variant_def.payload {
                    HashMap::from_iter([
                        ("type".into(), original_payload.payload_type),
                        ("value".into(), original_payload.value),
                    ])
                } else {
                    HashMap::new()
                };

                Variant {
                    name: variant_def.name.clone(),
                    payload,
                    enabled: variant_def.enabled,
                    feature_enabled,
                }
            })
            .unwrap_or_else(|| Variant::disabled(feature_enabled))
    }

    pub fn is_enabled(&self, feature_enum: F, context: Option<&Context>, default: bool) -> bool {
        let feature_name = serde_plain::to_string(&feature_enum).expect("bad enum");
        self.is_enabled_str(&feature_name, context, default)
    }

    pub fn is_enabled_str(
        &self,
        feature_name: &str,
        context: Option<&Context>,
        default: bool,
    ) -> bool {
        trace!("is_enabled: feature_str {feature_name:?} default {default}, context {context:?}");
        let cache = self.cached_state();
        let Some(cache) = cache.as_ref() else {
            trace!("is_enabled: No API state");
            return default;
        };

        let context = context
            .map(|context| build_yggdrasil_context(context, feature_name))
            .unwrap_or_else(|| EnrichedContext {
                user_id: None,
                session_id: None,
                environment: None,
                app_name: None,
                current_time: None,
                remote_address: None,
                properties: None,
                external_results: None,
                toggle_name: feature_name.to_string(),
                runtime_hostname: None,
            });

        let enabled = cache.check_enabled(&context).unwrap_or(default);
        cache.count_toggle(feature_name, enabled);
        enabled
    }

    /// Memoize new features into the cached state
    ///
    /// Interior mutability is used, via the arc-swap crate.
    ///
    /// Note that this is primarily public to facilitate benchmarking;
    /// poll_for_updates is the usual way in which memoize will be called.
    pub fn memoize(
        &self,
        features: UpdateMessage,
    ) -> Result<Option<Metrics>, Box<dyn std::error::Error + Send + Sync>> {
        trace!("memoize: start");
        let mut engine_state = EngineState::default();
        engine_state.take_state(features);

        // Now we have the new cache compiled, swap it in.
        let old = self.cached_state.swap(Some(Arc::new(engine_state)));

        trace!("memoize: swapped memoized state in");

        let old_metrics = old
            .and_then(|old| Arc::try_unwrap(old).ok())
            .and_then(|mut state| state.get_metrics(Utc::now()))
            .map(|metrics_bucket| Metrics {
                app_name: self.app_name.clone(),
                instance_id: self.instance_id.clone(),
                connection_id: self.connection_id.clone(),
                bucket: metrics_bucket,
            });

        Ok(old_metrics)
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
        let endpoint = features_endpoint(&self.api_url);
        let metrics_endpoint = Metrics::endpoint(&self.api_url);
        self.polling.store(true, Ordering::Relaxed);
        loop {
            debug!("poll: retrieving features");
            match self
                .http
                .get_json::<UpdateMessage>(&endpoint, Some(self.interval))
                .await
            {
                Ok(features) => match self.memoize(features) {
                    Ok(None) => {}
                    Ok(Some(metrics)) => {
                        if !self.disable_metric_submission {
                            let mut metrics_uploaded = false;
                            let res = self
                                .http
                                .post_json(&metrics_endpoint, metrics, Some(self.interval))
                                .await;
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
                    Err(err) => {
                        warn!("poll: failed to memoize features: {:?}", err);
                    }
                },
                Err(err) => {
                    warn!("poll: failed to retrieve features: {:?}", err);
                }
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
            connection_id: self.connection_id.clone(),
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
            .post_json(&Registration::endpoint(&self.api_url), &registration, None)
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

#[cfg(test)]
mod tests {
    use std::collections::hash_map::HashMap;
    use std::collections::hash_set::HashSet;
    use std::default::Default;
    use std::hash::BuildHasher;
    use std::sync::Arc;

    use chrono::Utc;
    use enum_map::Enum;
    use maplit::hashmap;
    use serde::{Deserialize, Serialize};
    use unleash_types::client_features::{ClientFeature, ClientFeatures, Payload, Strategy};
    use unleash_types::client_metrics::MetricBucket;
    use unleash_yggdrasil::{EngineState, UpdateMessage};

    use super::ClientBuilder;
    use crate::client::Variant;
    use crate::context::Context;
    use crate::strategy;

    cfg_if::cfg_if! {
        if #[cfg(feature = "reqwest")] {
            use reqwest::Client as HttpClient;
        } else if #[cfg(feature = "reqwest-11")] {
            use reqwest_11::Client as HttpClient;
        } else {
            compile_error!("Cannot run test suite without a client enabled");
        }
    }

    fn features() -> UpdateMessage {
        UpdateMessage::FullResponse(ClientFeatures {
            version: 1,
            features: vec![
                ClientFeature {
                    description: Some("default".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "default".into(),
                    strategies: Some(vec![Strategy {
                        name: "default".into(),
                        sort_order: None,
                        segments: None,
                        constraints: None,
                        parameters: None,
                        variants: None,
                    }]),

                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("userWithId".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "userWithId".into(),
                    strategies: Some(vec![Strategy {
                        name: "userWithId".into(),
                        parameters: Some(hashmap!["userIds".into()=>"present".into()]),
                        sort_order: None,
                        segments: None,
                        constraints: None,
                        variants: None,
                    }]),

                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("userWithId+default".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "userWithId+default".into(),
                    strategies: Some(vec![
                        Strategy {
                            name: "userWithId".into(),
                            parameters: Some(hashmap!["userIds".into()=>"present".into()]),
                            sort_order: None,
                            segments: None,
                            constraints: None,
                            variants: None,
                        },
                        Strategy {
                            name: "default".into(),
                            sort_order: None,
                            segments: None,
                            constraints: None,
                            parameters: None,
                            variants: None,
                        },
                    ]),

                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("disabled".to_string()),
                    enabled: false,
                    created_at: None,
                    variants: None,
                    name: "disabled".into(),
                    strategies: Some(vec![Strategy {
                        name: "default".into(),
                        sort_order: None,
                        segments: None,
                        constraints: None,
                        parameters: None,
                        variants: None,
                    }]),

                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("nostrategies".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "nostrategies".into(),
                    strategies: Some(vec![]),

                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
            ],
            query: None,
            segments: None,
            meta: None,
        })
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

        c.memoize(f).unwrap();
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

        c.memoize(f).unwrap();
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

        let f = UpdateMessage::FullResponse(ClientFeatures {
            version: 1,
            features: vec![
                ClientFeature {
                    description: Some("default".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "default".into(),
                    strategies: Some(vec![Strategy {
                        name: "default".into(),
                        sort_order: None,
                        segments: None,
                        constraints: None,
                        parameters: None,
                        variants: None,
                    }]),
                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("reversed".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "reversed".into(),
                    strategies: Some(vec![Strategy {
                        name: "reversed".into(),
                        parameters: Some(hashmap!["userIds".into()=>"abc".into()]),
                        sort_order: None,
                        segments: None,
                        constraints: None,
                        variants: None,
                    }]),
                    feature_type: Some("release".into()),
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
            ],
            segments: None,
            query: None,
            meta: None,
        });
        client.memoize(f).unwrap();
        let present: Context = Context {
            user_id: Some("cba".into()),
            ..Default::default()
        };
        let missing: Context = Context {
            user_id: Some("abc".into()),
            ..Default::default()
        };
        // user cba should be present on reversed
        // assert!(client.is_enabled(UserFeatures::reversed, Some(&present), false));
        // // user abc should not
        // assert!(!client.is_enabled(UserFeatures::reversed, Some(&missing), false));
        // // adding custom strategies shouldn't disable built-in ones
        // // default should be enabled, no context needed
        // assert!(client.is_enabled(UserFeatures::default, None, false));
    }

    fn variant_features() -> UpdateMessage {
        UpdateMessage::FullResponse(ClientFeatures {
            version: 1,
            features: vec![
                ClientFeature {
                    description: Some("disabled".to_string()),
                    enabled: false,
                    created_at: None,
                    variants: None,
                    name: "disabled".into(),
                    strategies: Some(vec![]),
                    feature_type: None,
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("novariants".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "novariants".into(),
                    strategies: Some(vec![Strategy {
                        name: "default".into(),
                        sort_order: None,
                        segments: None,
                        constraints: None,
                        parameters: None,
                        variants: None,
                    }]),
                    feature_type: None,
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("one".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: Some(vec![unleash_types::client_features::Variant {
                        name: "variantone".into(),
                        weight: 100,
                        payload: Some(Payload {
                            payload_type: "string".into(),
                            value: "val1".into(),
                        }),
                        overrides: None,
                        weight_type: None,
                        stickiness: None,
                    }]),
                    name: "one".into(),
                    strategies: Some(vec![]),
                    feature_type: None,
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("two".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: Some(vec![
                        unleash_types::client_features::Variant {
                            name: "variantone".into(),
                            weight: 50,
                            payload: Some(Payload {
                                payload_type: "string".into(),
                                value: "val1".into(),
                            }),
                            overrides: None,
                            weight_type: None,
                            stickiness: None,
                        },
                        unleash_types::client_features::Variant {
                            name: "varianttwo".into(),
                            weight: 50,
                            payload: Some(Payload {
                                payload_type: "string".into(),
                                value: "val2".into(),
                            }),
                            overrides: None,
                            weight_type: None,
                            stickiness: None,
                        },
                    ]),
                    name: "two".into(),
                    strategies: Some(vec![]),
                    feature_type: None,
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
                ClientFeature {
                    description: Some("nostrategies".to_string()),
                    enabled: true,
                    created_at: None,
                    variants: None,
                    name: "nostrategies".into(),
                    strategies: Some(vec![]),
                    feature_type: None,
                    last_seen_at: None,
                    stale: None,
                    impression_data: None,
                    project: None,
                    dependencies: None,
                },
            ],
            segments: None,
            query: None,
            meta: None,
        })
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

        c.memoize(f).unwrap();

        // disabled should be disabled
        let variant = Variant::disabled(false);
        assert_eq!(
            variant,
            c.get_variant(UserFeatures::disabled, &Context::default())
        );

        // enabled no variants should get the disabled variant
        let variant = Variant::disabled(true);
        assert_eq!(
            variant,
            c.get_variant(UserFeatures::novariants, &Context::default())
        );

        // // One variant
        let variant = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
            feature_enabled: true,
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
        let variant1 = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
            feature_enabled: true,
        };
        let variant2 = Variant {
            name: "varianttwo".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val2".into()
            ],
            enabled: true,
            feature_enabled: true,
        };
        assert_eq!(variant1, c.get_variant(UserFeatures::two, &uid1));
        assert_eq!(variant2, c.get_variant(UserFeatures::two, &session1));
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

        c.memoize(f).unwrap();

        // disabled should be disabled
        let variant = Variant::disabled(false);
        assert_eq!(variant, c.get_variant_str("disabled", &Context::default()));

        // enabled no variants should get the disabled variant
        let variant = Variant::disabled(true);
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
            feature_enabled: true,
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
        let variant1 = Variant {
            name: "variantone".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val1".into()
            ],
            enabled: true,
            feature_enabled: true,
        };
        let variant2 = Variant {
            name: "varianttwo".to_string(),
            payload: hashmap![
                "type".into()=>"string".into(),
                "value".into()=>"val2".into()
            ],
            enabled: true,
            feature_enabled: true,
        };
        assert_eq!(variant1, c.get_variant_str("two", &uid1));
        assert_eq!(variant2, c.get_variant_str("two", &session1));
    }

    #[test]
    fn variant_metrics() {
        macro_rules! feature_name {
            ($e:expr) => {
                serde_plain::to_string(&$e).unwrap()
            };
        }

        fn get_variant_count(metrics: &MetricBucket, toggle: &str, variant: &str) -> u32 {
            metrics
                .toggles
                .get(toggle)
                .and_then(|toggle| toggle.variants.get(variant))
                .cloned()
                .unwrap_or_else(|| {
                    panic!("Missing count for variant '{variant}' in toggle '{toggle}'")
                })
        }

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

        c.memoize(f).unwrap();

        c.get_variant(UserFeatures::disabled, &Context::default());
        c.get_variant(UserFeatures::novariants, &Context::default());

        let session1: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };

        let user_id1: Context = Context {
            user_id: Some("user_id1".into()),
            ..Default::default()
        };
        c.get_variant(UserFeatures::two, &session1);
        c.get_variant(UserFeatures::two, &user_id1);

        let metrics = c
            .cached_state
            .swap(Some(Arc::new(EngineState::default())))
            .and_then(|old| Arc::try_unwrap(old).ok())
            .and_then(|mut state| state.get_metrics(Utc::now()))
            .expect("Really expected to see some metrics here");

        let no_variants = feature_name!(UserFeatures::novariants);
        let disabled = feature_name!(UserFeatures::disabled);
        let two = feature_name!(UserFeatures::two);

        assert_eq!(get_variant_count(&metrics, &no_variants, "disabled"), 1);
        assert_eq!(get_variant_count(&metrics, &disabled, "disabled"), 1);
        assert_eq!(get_variant_count(&metrics, &two, "variantone"), 1);
        assert_eq!(get_variant_count(&metrics, &two, "varianttwo"), 1);
    }
}
