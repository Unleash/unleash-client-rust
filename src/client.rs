// Copyright 2020 Cognite AS
//! The primary interface for users of the library.
use std::collections::hash_map::HashMap;
use std::default::Default;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use chrono::Utc;
use futures_timer::Delay;
use log::{debug, trace, warn};

use crate::api::{Feature, Features, Metrics, MetricsBucket, Registration};
use crate::context::Context;
use crate::http::HTTP;
use crate::strategy;

pub struct ClientBuilder {
    interval: u64,
    strategies: HashMap<String, strategy::Strategy>,
}

impl ClientBuilder {
    pub fn into_client<C: http_client::HttpClient + Default>(
        self,
        api_url: &str,
        app_name: &str,
        instance_id: &str,
        authorization: Option<String>,
    ) -> Result<Client<C>, http_client::Error> {
        Ok(Client {
            api_url: api_url.into(),
            app_name: app_name.into(),
            instance_id: instance_id.into(),
            interval: self.interval,
            polling: AtomicBool::new(false),
            http: HTTP::new(app_name.into(), instance_id.into(), authorization)?,
            cached_state: ArcSwapOption::from(None),
            strategies: Mutex::new(self.strategies),
        })
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

struct CachedFeature {
    strategies: Arc<Vec<strategy::Evaluate>>,
    // unknown features are tracked for metrics (so the server can see that they
    // are being used). They require specific logic (see is_enabled).
    unknown: bool,
    // Tracks metrics during a refresh interval. If the AtomicBool updates show
    // to be a contention point then thread-sharded counters with a gather phase
    // on submission will be the next logical progression.
    enabled: AtomicU64,
    disabled: AtomicU64,
}

struct CachedState {
    start: chrono::DateTime<chrono::Utc>,
    features: HashMap<String, CachedFeature>,
}

pub struct Client<C: http_client::HttpClient> {
    api_url: String,
    app_name: String,
    instance_id: String,
    interval: u64,
    polling: AtomicBool,
    http: HTTP<C>,
    // known strategies: strategy_name : memoiser
    strategies: Mutex<HashMap<String, strategy::Strategy>>,
    // memoised state: feature_name: [callback, callback, ...]
    cached_state: ArcSwapOption<CachedState>,
}

unsafe impl<C: http_client::HttpClient + std::default::Default> Sync for Client<C> {}

impl<C: http_client::HttpClient + std::default::Default> Client<C> {
    pub fn new(
        api_url: &str,
        app_name: &str,
        instance_id: &str,
        authorization: Option<String>,
    ) -> Result<Self, http_client::Error> {
        let builder = ClientBuilder::default();
        Ok(Self {
            api_url: api_url.into(),
            app_name: app_name.into(),
            instance_id: instance_id.into(),
            interval: 15000,
            polling: AtomicBool::new(false),
            http: HTTP::new(app_name.into(), instance_id.into(), authorization)?,
            cached_state: ArcSwapOption::from(None),
            strategies: Mutex::new(builder.strategies),
        })
    }

    pub fn is_enabled(&self, feature_name: &str, context: Option<&Context>, default: bool) -> bool {
        trace!(
            "is_enabled: feature {} default {}, context {:?}",
            feature_name,
            default,
            context
        );
        let cache = self.cached_state.load();
        let cache = if let Some(cache) = &*cache {
            cache
        } else {
            // No API state loaded
            trace!("is_enabled: No API state");
            return false;
        };
        if let Some(feature) = cache.features.get(feature_name) {
            let default_context: Context = Default::default();
            let context = context.unwrap_or(&default_context);
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
            if feature.unknown {
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
                .rcu(|cached_state: &Option<Arc<CachedState>>| {
                    // Did someone swap None in ?
                    if let Some(cached_state) = cached_state {
                        let cached_state = cached_state.clone();
                        if let Some(feature) = cached_state.features.get(feature_name) {
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
                        } else {
                            // still not present; add it
                            let stub_feature = CachedFeature {
                                disabled: AtomicU64::new(if default { 0 } else { 1 }),
                                enabled: AtomicU64::new(if default { 1 } else { 0 }),
                                unknown: true,
                                strategies: Arc::new(vec![]),
                            };
                            // Build up a new cached state
                            let mut new_state = CachedState {
                                start: cached_state.start,
                                features: HashMap::new(),
                            };
                            for (name, feature) in &cached_state.features {
                                new_state.features.insert(
                                    name.clone(),
                                    CachedFeature {
                                        disabled: AtomicU64::new(
                                            feature.disabled.load(Ordering::Relaxed),
                                        ),
                                        enabled: AtomicU64::new(
                                            feature.enabled.load(Ordering::Relaxed),
                                        ),
                                        unknown: feature.unknown,
                                        strategies: feature.strategies.clone(),
                                    },
                                );
                            }
                            new_state.features.insert(feature_name.into(), stub_feature);
                        }
                        Some(cached_state)
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
        let mut cached_features: HashMap<String, CachedFeature> = HashMap::new();
        // HashMap<String, Vec<Box<strategy::Evaluate>>> = HashMap::new();
        for feature in features {
            if !feature.enabled {
                // no strategies == return false per the unleash example code;
                let strategies = Arc::new(vec![]);
                let cached_feature = CachedFeature {
                    strategies,
                    disabled: AtomicU64::new(0),
                    enabled: AtomicU64::new(0),
                    unknown: false,
                };
                cached_features.insert(feature.name.clone(), cached_feature);
                continue;
            }
            // TODO add variant support
            let mut strategies = vec![];
            for api_strategy in feature.strategies {
                if let Some(code_strategy) = source_strategies.get(&api_strategy.name) {
                    strategies.push(code_strategy(api_strategy.parameters));
                }
                // Graceful degradation: ignore this unknown strategy.
                // TODO: add a logging layer and log it.
            }
            let cached_feature = CachedFeature {
                strategies: Arc::new(strategies),
                disabled: AtomicU64::new(0),
                enabled: AtomicU64::new(0),
                unknown: false,
            };
            cached_features.insert(feature.name.clone(), cached_feature);
        }
        let new_cache = CachedState {
            start: now,
            features: cached_features,
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
            for (name, feature) in &old.features {
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
                bucket: bucket,
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
                    Err(_) => {
                        warn!("poll: failed to memoize features");
                    }
                }
            } else {
                warn!("poll: failed to retrieve features");
            }
            Delay::new(Duration::from_millis(self.interval)).await;
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

    use maplit::hashmap;

    use super::{Client, ClientBuilder};
    use crate::api::{Feature, Features, Strategy};
    use crate::context::Context;
    use crate::strategy;

    #[test]
    fn test_memoization() {
        let _ = simple_logger::init();
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
            ],
        };
        let c = Client::<http_client::native::NativeClient>::new(
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
        // unknown features should honour the default
        assert_eq!(false, c.is_enabled("unknown", None, false));
        assert_eq!(true, c.is_enabled("unknown", None, true));
        // default should be enabled, no context needed
        assert_eq!(true, c.is_enabled("default", None, false));
        // user present should be present on userWithId
        assert_eq!(true, c.is_enabled("userWithId", Some(&present), false));
        // user missing should not
        assert_eq!(false, c.is_enabled("userWithId", Some(&missing), false));
        // user missing should be present on userWithId+default
        assert_eq!(
            true,
            c.is_enabled("userWithId+default", Some(&missing), false)
        );
        // disabled should be disabled
        assert_eq!(false, c.is_enabled("disabled", None, true));
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
        let _ = simple_logger::init();
        let client = ClientBuilder::default()
            .strategy("reversed", Box::new(&_reversed_uids))
            .into_client::<http_client::native::NativeClient>(
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
        assert_eq!(true, client.is_enabled("reversed", Some(&present), false));
        // user abc should not
        assert_eq!(false, client.is_enabled("reversed", Some(&missing), false));
        // adding custom strategies shouldn't disable built-in ones
        // default should be enabled, no context needed
        assert_eq!(true, client.is_enabled("default", None, false));
    }
}
