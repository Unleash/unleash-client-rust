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

pub struct CustomStrategyHandler {
    strategy_definitions: ArcSwap<HashMap<String, Vec<(String, Option<HashMap<String, String>>)>>>,
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
            println!("My results {:#?}", results);
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
