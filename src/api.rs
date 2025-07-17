// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/api/client/features>
use std::collections::HashMap;
use std::default::Default;

use crate::version::get_sdk_version;
use chrono::{DateTime, FixedOffset, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Features {
    pub version: u8,
    pub features: Vec<Feature>,
}

impl Features {
    pub fn endpoint(api_url: &str) -> String {
        format!("{}/client/features", api_url.trim_end_matches('/'))
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Feature {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub enabled: bool,
    pub strategies: Vec<Strategy>,
    pub variants: Option<Vec<Variant>>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Strategy {
    pub constraints: Option<Vec<Constraint>>,
    pub name: String,
    pub parameters: Option<HashMap<String, String>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Constraint {
    #[serde(rename = "contextName")]
    pub context_name: String,
    #[serde(flatten)]
    pub expression: ConstraintExpression,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "operator", content = "values")]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub enum ConstraintExpression {
    //. Dates
    #[serde(rename = "DATE_AFTER")]
    DateAfter(DateTime<FixedOffset>),
    #[serde(rename = "DATE_BEFORE")]
    DateBefore(DateTime<FixedOffset>),

    // In
    #[serde(rename = "IN")]
    In(Vec<String>),
    #[serde(rename = "NOT_IN")]
    NotIn(Vec<String>),

    // Numbers
    #[serde(rename = "NUM_EQ")]
    NumEq(String),
    #[serde(rename = "NUM_GT")]
    NumGT(String),
    #[serde(rename = "NUM_GTE")]
    NumGTE(String),
    #[serde(rename = "NUM_LT")]
    NumLT(String),
    #[serde(rename = "NUM_LTE")]
    NumLTE(String),

    // Semver
    #[serde(rename = "SEMVER_EQ")]
    SemverEq(Version),
    #[serde(rename = "SEMVER_GT")]
    SemverGT(Version),
    #[serde(rename = "SEMVER_LT")]
    SemverLT(Version),

    // String
    #[serde(rename = "STR_CONTAINS")]
    StrContains(Vec<String>),
    #[serde(rename = "STR_STARTS_WITH")]
    StrStartsWith(Vec<String>),
    #[serde(rename = "STR_ENDS_WITH")]
    StrEndsWith(Vec<String>),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Variant {
    pub name: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub weight: u16,
    pub payload: Option<HashMap<String, String>>,
    pub overrides: Option<Vec<VariantOverride>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
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
    #[serde(rename = "connectionId")]
    pub connection_id: String,
    #[serde(rename = "sdkVersion")]
    pub sdk_version: String,
    pub strategies: Vec<String>,
    pub started: chrono::DateTime<chrono::Utc>,
    pub interval: u64,
}

impl Registration {
    pub fn endpoint(api_url: &str) -> String {
        format!("{}/client/register", api_url.trim_end_matches('/'))
    }
}

impl Default for Registration {
    fn default() -> Self {
        Self {
            app_name: "".into(),
            instance_id: "".into(),
            connection_id: "".into(),
            sdk_version: get_sdk_version().into(),
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
    #[serde(rename = "connectionId")]
    pub connection_id: String,
    pub bucket: MetricsBucket,
}

impl Metrics {
    pub fn endpoint(api_url: &str) -> String {
        format!("{}/client/metrics", api_url.trim_end_matches('/'))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ToggleMetrics {
    pub yes: u64,
    pub no: u64,
    pub variants: HashMap<String, u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MetricsBucket {
    pub start: chrono::DateTime<chrono::Utc>,
    pub stop: chrono::DateTime<chrono::Utc>,
    pub toggles: HashMap<String, ToggleMetrics>,
}

fn deserialize_number_from_string<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr + serde::Deserialize<'de>,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt<T> {
        String(String),
        Number(T),
    }

    match StringOrInt::<T>::deserialize(deserializer)? {
        StringOrInt::String(s) => s.parse::<T>().map_err(serde::de::Error::custom),
        StringOrInt::Number(i) => Ok(i),
    }
}

#[cfg(test)]
mod tests {

    use super::{Features, Metrics, Registration};

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
    fn parse_null_feature_doc() -> Result<(), serde_json::Error> {
        let data = r#"
    {
      "version": 1,
      "features": [
      {
        "name": "F1",
        "description": null,
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
      }
      ]
    }
    "#;
        let parsed: super::Features = serde_json::from_str(data)?;
        assert_eq!(1, parsed.version);
        Ok(())
    }

    #[test]
    fn test_parse_variant_with_str_weight() -> Result<(), serde_json::Error> {
        let data = r#"
      {"name":"Foo","weight":"50","payload":{"type":"string","value":"bar"}}
      "#;
        let parsed: super::Variant = serde_json::from_str(data)?;
        assert_eq!(50, parsed.weight);
        Ok(())
    }

    #[test]
    fn test_registration_customisation() {
        Registration {
            app_name: "test-suite".into(),
            instance_id: "test".into(),
            connection_id: "test".into(),
            strategies: vec!["default".into()],
            interval: 5000,
            ..Default::default()
        };
    }

    #[test]
    fn test_endpoints_handle_trailing_slashes() {
        assert_eq!(
            Registration::endpoint("https://localhost:4242/api"),
            "https://localhost:4242/api/client/register"
        );
        assert_eq!(
            Registration::endpoint("https://localhost:4242/api/"),
            "https://localhost:4242/api/client/register"
        );

        assert_eq!(
            Features::endpoint("https://localhost:4242/api"),
            "https://localhost:4242/api/client/features"
        );
        assert_eq!(
            Features::endpoint("https://localhost:4242/api/"),
            "https://localhost:4242/api/client/features"
        );

        assert_eq!(
            Metrics::endpoint("https://localhost:4242/api"),
            "https://localhost:4242/api/client/metrics"
        );
        assert_eq!(
            Metrics::endpoint("https://localhost:4242/api/"),
            "https://localhost:4242/api/client/metrics"
        );
    }
}
