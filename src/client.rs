/// The primary interface for users of the library.
use std::collections::hash_map::HashMap;
use std::default::Default;
use std::sync::Arc;

use log::{debug, info, trace, warn};

use arc_swap::ArcSwapOption;

use crate::api::Feature;
use crate::context::Context;
use crate::http::HTTP;
use crate::strategy;

pub struct Client<'a, C: http_client::HttpClient> {
    http: HTTP<C>,
    // known strategies: strategy_name : memoiser
    strategies: HashMap<String, &'a strategy::Strategy<'a>>,
    // memoised state: feature_name: [callback, callback, ...]
    cached_state: ArcSwapOption<HashMap<String, Vec<Box<strategy::Evaluate>>>>,
}

impl<'a, C: http_client::HttpClient + std::default::Default> Client<'a, C> {
    pub fn new(
        app_name: String,
        instance_id: String,
        authorization: Option<String>,
    ) -> Result<Self, http_client::Error> {
        let mut strategies: HashMap<String, &'a strategy::Strategy<'a>> = HashMap::new();
        strategies.insert("default".into(), &strategy::default);
        strategies.insert("applicationHostname".into(), &strategy::hostname);
        strategies.insert("default".into(), &strategy::default);
        strategies.insert("gradualRolloutRandom".into(), &strategy::random);
        strategies.insert("gradualRolloutSessionId".into(), &strategy::session_id);
        strategies.insert("gradualRolloutUserId".into(), &strategy::user_id);
        strategies.insert("remoteAddress".into(), &strategy::remote_address);
        strategies.insert("userWithId".into(), &strategy::user_with_id);
        strategies.insert("flexibleRollout".into(), &strategy::flexible_rollout);
        Ok(Self {
            http: HTTP::new(app_name, instance_id, authorization)?,
            cached_state: ArcSwapOption::from(None),
            strategies,
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
        if let Some(memos) = cache.get(feature_name) {
            let default_context: Context = Default::default();
            let context = context.unwrap_or(&default_context);
            for memo in memos {
                if memo(context) {
                    debug!(
                        "is_enabled: feature {} enabled by memo {:p}, context {:?}",
                        feature_name, memo, context
                    );
                    return true;
                } else {
                    trace!(
                        "is_enabled: feature {} not enabled by memo {:p}, context {:?}",
                        feature_name,
                        memo,
                        context
                    );
                }
            }
            false
        } else {
            trace!(
                "is_enabled: Unknown feature {}, using default {}",
                feature_name,
                default
            );
            default
        }
    }

    pub fn memoize(&mut self, features: Vec<Feature>) -> Result<(), Box<dyn std::error::Error>> {
        let mut new_cache: HashMap<String, Vec<Box<strategy::Evaluate>>> = HashMap::new();
        for feature in features {
            if !feature.enabled {
                let memos = vec![Box::new(_disabled) as Box<dyn Fn(&Context) -> bool>];
                new_cache.insert(feature.name.clone(), memos);
                continue;
            }
            // TODO add variant support
            let mut memos = vec![];
            for api_strategy in feature.strategies {
                if let Some(code_strategy) = self.strategies.get(&api_strategy.name) {
                    memos.push(code_strategy(api_strategy.parameters));
                }
                // Graceful degradation: ignore this unknown strategy.
                // TODO: add a logging layer and log it.
                // TODO: add a metrics layer and emit metrics for it.
            }
            new_cache.insert(feature.name.clone(), memos);
        }
        self.cached_state.store(Some(Arc::new(new_cache)));
        Ok(())
    }
}

fn _disabled(_: &Context) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;

    use super::Client;
    use crate::api::{Feature, Features, Strategy};
    use crate::context::Context;
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
        let mut c =
            Client::<http_client::native::NativeClient>::new("foo".into(), "test".into(), None)
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
}
