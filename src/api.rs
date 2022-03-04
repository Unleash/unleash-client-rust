// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/api/client/features>
use std::collections::HashMap;
use std::default::Default;

use chrono::Utc;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};

use crate::{Context, Evaluate};

#[derive(Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Features {
    pub version: u8,
    pub features: Vec<Feature>,
}

impl Features {
    pub fn endpoint(api_url: &str) -> String {
        format!("{}/client/features", api_url)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Feature {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub enabled: bool,
    pub strategies: Vec<Strategy>,
    pub variants: Option<Vec<Variant>>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Strategy {
    pub constraints: Option<Vec<Constraint>>,
    pub name: String,
    pub parameters: Option<HashMap<String, String>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum StrConstraint {
    StrEndsWith(Vec<String>, bool),
    StrStartsWith(Vec<String>, bool),
    StrContains(Vec<String>, bool),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum NumericConstraint {
    NumEq(String),
    NumGt(String),
    NumGte(String),
    NumLt(String),
    NumLte(String),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum DateConstraint {
    DateAfter(String),
    DateBefore(String),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum SemverConstraint {
    SemverEq(String),
    SemverGt(String),
    SemverLt(String),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum ContainsConstraint {
    In(Vec<String>),
    NotIn(Vec<String>),
}

#[enum_dispatch]
#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum ConstraintExpression {
    NumericConstraint,
    StrConstraint,
    DateConstraint,
    SemverConstraint,
    ContainsConstraint,
}

#[enum_dispatch(ConstraintExpression)]
pub trait EvaluatorConstructor {
    fn yield_evaluator(self, getter: crate::strategy::Expr) -> Evaluate;
}

pub trait Expression: Fn(&Context) -> Option<&String> {
    fn clone_boxed(&self) -> Box<dyn Expression + Send + Sync + 'static>;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Constraint {
    #[serde(rename = "contextName")]
    pub context_name: String,
    pub inverted: Option<bool>,
    #[serde(flatten)]
    pub expression: ConstraintExpression,
}

// #[derive(Clone, Serialize, Deserialize, Debug)]
// #[serde(tag = "operator")]
// // #[serde(deny_unknown_fields)]
// pub enum ConstraintExpression {
//     #[serde(rename = "IN")]
//     In { values: Vec<String> },
//     #[serde(rename = "NOT_IN")]
//     NotIn { values: Vec<String> },
//     #[serde(rename = "STR_ENDS_WITH")]
//     StrEndsWith {
//         values: Vec<String>,
//         #[serde(rename = "caseInsensitive")]
//         case_insensitive: Option<bool>,
//     },
//     #[serde(rename = "STR_STARTS_WITH")]
//     StrStartsWith {
//         values: Vec<String>,
//         #[serde(rename = "caseInsensitive")]
//         case_insensitive: Option<bool>,
//     },
//     #[serde(rename = "STR_CONTAINS")]
//     StrContains {
//         values: Vec<String>,
//         #[serde(rename = "caseInsensitive")]
//         case_insensitive: Option<bool>,
//     },
//     #[serde(rename = "NUM_EQ")]
//     NumEq { value: String },
//     #[serde(rename = "NUM_GT")]
//     NumGt { value: String },
//     #[serde(rename = "NUM_GTE")]
//     NumGte { value: String },
//     #[serde(rename = "NUM_LT")]
//     NumLt { value: String },
//     #[serde(rename = "NUM_LTE")]
//     NumLte { value: String },
//     #[serde(rename = "DATE_AFTER")]
//     DateAfter { value: String },
//     #[serde(rename = "DATE_BEFORE")]
//     DateBefore { value: String },
//     #[serde(rename = "SEMVER_EQ")]
//     SemverEq { value: String },
//     #[serde(rename = "SEMVER_GT")]
//     SemverGt { value: String },
//     #[serde(rename = "SEMVER_LT")]
//     SemverLt { value: String },
// }

#[derive(Clone, Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Variant {
    pub name: String,
    pub weight: u8,
    pub payload: Option<HashMap<String, String>>,
    pub overrides: Option<Vec<VariantOverride>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct VariantOverride {
    #[serde(rename = "contextName")]
    pub context_name: String,
    pub values: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Registration {
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(rename = "instanceId")]
    pub instance_id: String,
    #[serde(rename = "sdkVersion")]
    pub sdk_version: String,
    pub strategies: Vec<String>,
    pub started: chrono::DateTime<chrono::Utc>,
    pub interval: u64,
}

impl Registration {
    pub fn endpoint(api_url: &str) -> String {
        format!("{}/client/register", api_url)
    }
}

impl Default for Registration {
    fn default() -> Self {
        Self {
            app_name: "".into(),
            instance_id: "".into(),
            sdk_version: "unleash-client-rust-0.1.0".into(),
            strategies: vec![],
            started: Utc::now(),
            interval: 15 * 1000,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Metrics {
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(rename = "instanceId")]
    pub instance_id: String,
    pub bucket: MetricsBucket,
}

impl Metrics {
    pub fn endpoint(api_url: &str) -> String {
        format!("{}/client/metrics", api_url)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MetricsBucket {
    pub start: chrono::DateTime<chrono::Utc>,
    pub stop: chrono::DateTime<chrono::Utc>,
    /// name: "yes"|"no": count
    pub toggles: HashMap<String, HashMap<String, u64>>,
}

#[cfg(test)]
mod tests {
    use super::Registration;
    use super::*;

    #[test]
    fn parses_advanced_constraint_structure() -> Result<(), serde_json::Error> {
        let data = r#"
    {
      "contextName": "customField",
      "operator": "STR_STARTS_WITH",
      "values": ["some-string"]
    }"#;
        let _: super::Constraint = serde_json::from_str(data)?;

        let data = r#"
    {
      "contextName": "customField",
      "operator": "NUM_GTE",
      "value": "7"
    }"#;
        let _: super::Constraint = serde_json::from_str(data)?;

        let data = r#"
    {
      "contextName": "customField",
      "operator": "STR_STARTS_WITH",
      "values": ["some-string"],
      "caseInsensitive": true,
      "inverted": true
    }"#;
        let _: super::Constraint = serde_json::from_str(data)?;

        Ok(())
    }

    #[test]
    fn parse_reference_doc() -> Result<(), serde_json::Error> {
        let data = r#"
    {
      "version": 1,
      "features": [
      {
        "name": "F1",
        "description": "Default Strategy, enabledoff, variants",
        "enabled": false,
        "strategies": [
        {
          "name": "default"
        }
        ],
        "variants":[
        {"name":"Foo","weight":50,"payload":{"type":"string","value":"bar"}},
        {"name":"Bar","weight":50,"overrides":[{"contextName":"userId","values":["robert"]}]}
        ],
        "createdAt": "2020-04-28T07:26:27.366Z"
      },
      {
        "name": "F2",
        "description": "customStrategy+params, enabled",
        "enabled": true,
        "strategies": [
        {
          "name": "customStrategy",
          "parameters": {
            "strategyParameter": "data,goes,here"
          }
        }
        ],
        "variants": null,
        "createdAt": "2020-01-12T15:05:11.462Z"
      },
      {
        "name": "F3",
        "description": "two strategies",
        "enabled": true,
        "strategies": [
        {
          "name": "customStrategy",
          "parameters": {
            "strategyParameter": "data,goes,here"
          }
        },
        {
          "name": "default",
          "parameters": {}
        }
        ],
        "variants": null,
        "createdAt": "2019-09-30T09:00:39.282Z"
      },
      {
        "name": "F4",
        "description": "Multiple params",
        "enabled": true,
        "strategies": [
        {
          "name": "customStrategy",
          "parameters": {
            "p1": "foo",
            "p2": "bar"
          }
        }
        ],
        "variants": null,
        "createdAt": "2020-03-17T01:07:25.713Z"
      }
      ]
    }
    "#;
        let parsed: super::Features = serde_json::from_str(data)?;
        assert_eq!(1, parsed.version);
        Ok(())
    }

    #[test]
    fn test_registration_customisation() {
        Registration {
            app_name: "test-suite".into(),
            instance_id: "test".into(),
            strategies: vec!["default".into()],
            interval: 5000,
            ..Default::default()
        };
    }
}
