// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/user_guide/activation_strategy>

use std::{collections::hash_map::HashMap, sync::Arc};

use arc_swap::ArcSwap;
use unleash_types::client_features::{ClientFeature, DeltaEvent};
use unleash_yggdrasil::{UpdateMessage, KNOWN_STRATEGIES};

use crate::context::Context;

/// Memoise feature state for a strategy.
pub type Strategy =
    Box<dyn Fn(Option<HashMap<String, String>>) -> Evaluate + Sync + Send + 'static>;
/// Apply memoised state to a context.
pub trait Evaluator: Fn(&Context) -> bool {
    fn clone_boxed(&self) -> Box<dyn Evaluator + Send + Sync + 'static>;
}
pub type Evaluate = Box<dyn Evaluator + Send + Sync + 'static>;

impl<T> Evaluator for T
where
    T: 'static + Clone + Sync + Send + Fn(&Context) -> bool,
{
    fn clone_boxed(&self) -> Box<dyn Evaluator + Send + Sync + 'static> {
        Box::new(T::clone(self))
    }
}

impl Clone for Box<dyn Evaluator + Send + Sync + 'static> {
    fn clone(&self) -> Self {
        self.as_ref().clone_boxed()
    }
}

type StrategyDefinitions = HashMap<String, Vec<(String, Option<HashMap<String, String>>)>>;

pub struct CustomStrategyHandler {
    strategy_definitions: ArcSwap<StrategyDefinitions>,
    strategy_implementations: HashMap<String, Strategy>,
}

impl CustomStrategyHandler {
    pub fn new(strategy_implementations: HashMap<String, Strategy>) -> Self {
        Self {
            strategy_definitions: ArcSwap::from_pointee(HashMap::new()),
            strategy_implementations,
        }
    }

    pub fn get_known_strategies(&self) -> Vec<String> {
        self.strategy_implementations
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn evaluate_custom_strategies(
        &self,
        toggle_name: &str,
        context: &Context,
    ) -> Option<HashMap<String, bool>> {
        let defs = self.strategy_definitions.load();

        defs.get(toggle_name).map(|definitions| {
            let mut results = HashMap::new();
            for (index, (strategy_name, parameters)) in definitions.iter().enumerate() {
                let key = format!("customStrategy{}", index + 1);
                let result = self
                    .strategy_implementations
                    .get(strategy_name)
                    .map(|strategy| strategy(parameters.clone())(context))
                    .unwrap_or(false);
                results.insert(key, result);
            }
            results
        })
    }

    pub fn update_strategies(&self, message: &UpdateMessage) {
        let mut new_strategies = HashMap::new();

        let mut collect = |feature: &ClientFeature| {
            if let Some(strategies) = &feature.strategies {
                let custom: Vec<_> = strategies
                    .iter()
                    .filter(|s| !KNOWN_STRATEGIES.contains(&s.name.as_str()))
                    .map(|s| (s.name.clone(), s.parameters.clone()))
                    .collect();

                if !custom.is_empty() {
                    new_strategies.insert(feature.name.clone(), custom);
                }
            }
        };

        match message {
            UpdateMessage::FullResponse(client_features) => {
                client_features.features.iter().for_each(&mut collect);
            }
            UpdateMessage::PartialUpdate(update_message) => {
                for event in &update_message.events {
                    match event {
                        DeltaEvent::FeatureUpdated { feature, .. } => collect(feature),
                        DeltaEvent::Hydration { features, .. } => {
                            features.iter().for_each(&mut collect);
                        }
                        _ => {}
                    }
                }
            }
        }

        self.strategy_definitions.store(Arc::new(new_strategies));
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::hash_map, hash::BuildHasher};

    use maplit::hashmap;
    use unleash_types::client_features::ClientFeatures;

    use super::*;

    fn true_if_user_7<S: BuildHasher>(_: Option<HashMap<String, String, S>>) -> Evaluate {
        Box::new(move |context: &Context| -> bool {
            context
                .user_id
                .as_ref()
                .map_or(false, |user_id| user_id == "7")
        })
    }

    #[test]
    fn test_custom_strategy_handler() {
        let strategy: Strategy = Box::new(true_if_user_7::<hash_map::RandomState>);

        let handler = CustomStrategyHandler::new(hashmap! {
            "customStrategy1".to_string() =>  strategy,
        });

        handler.update_strategies(&UpdateMessage::FullResponse(ClientFeatures {
            features: vec![ClientFeature {
                name: "featWithStrategy".to_string(),
                description: Some("Test strategy".to_string()),
                enabled: true,
                strategies: Some(vec![unleash_types::client_features::Strategy {
                    name: "customStrategy1".to_string(),
                    parameters: None,
                    sort_order: None,
                    segments: None,
                    constraints: None,
                    variants: None,
                }]),
                feature_type: Some("release".into()),
                ..Default::default()
            }],
            version: 2,
            segments: None,
            query: None,
            meta: None,
        }));

        let context = Context {
            user_id: Some("7".to_string()),
            ..Default::default()
        };

        let result = handler.evaluate_custom_strategies("featWithStrategy", &context);
        assert_eq!(
            result,
            Some(hashmap! { "customStrategy1".to_string() => true })
        );

        let context = Context {
            user_id: Some("8".to_string()),
            ..Default::default()
        };

        let result = handler.evaluate_custom_strategies("featWithStrategy", &context);
        assert_eq!(
            result,
            Some(hashmap! { "customStrategy1".to_string() => false })
        );
        let result = handler.evaluate_custom_strategies("nonExistentFeature", &context);
        assert!(result.is_none());
    }
}
