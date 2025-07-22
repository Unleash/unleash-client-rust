// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/api/client/features>
use std::collections::HashMap;
use std::default::Default;

use crate::version::get_sdk_version;
use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Constraint {
    pub context_name: String,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(default)]
    pub inverted: bool,
    #[serde(flatten)]
    pub expression: ConstraintExpression,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "operator")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConstraintExpression {
    // Dates
    DateAfter {
        value: DateTime<Utc>,
    },
    DateBefore {
        value: DateTime<Utc>,
    },

    // In
    In {
        values: Vec<String>,
    },
    NotIn {
        values: Vec<String>,
    },

    // Numbers
    NumEq {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        value: f64,
    },
    #[serde(rename = "NUM_GT")]
    NumGT {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        value: f64,
    },
    #[serde(rename = "NUM_GTE")]
    NumGTE {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        value: f64,
    },
    #[serde(rename = "NUM_LT")]
    NumLT {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        value: f64,
    },
    #[serde(rename = "NUM_LTE")]
    NumLTE {
        #[serde(deserialize_with = "deserialize_number_from_string")]
        value: f64,
    },

    // Semver
    SemverEq {
        value: Version,
    },
    #[serde(rename = "SEMVER_GT")]
    SemverGT {
        value: Version,
    },
    #[serde(rename = "SEMVER_LT")]
    SemverLT {
        value: Version,
    },

    // String
    StrContains {
        values: Vec<String>,
    },
    StrStartsWith {
        values: Vec<String>,
    },
    StrEndsWith {
        values: Vec<String>,
    },

    #[serde(untagged)]
    Unknown(Value),
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

    use std::str::FromStr;

    use chrono::{DateTime, FixedOffset};
    use semver::Version;

    use super::{Constraint, Features, Metrics, Registration};

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
    fn test_parse_constraint() -> Result<(), serde_json::Error> {
        use super::ConstraintExpression::*;
        let data = r#"[
          {
            "contextName": "appId",
            "operator": "IN",
            "values": [
                "app.known.name"
            ],
            "caseInsensitive": false,
            "inverted": false
          },
          {
            "contextName": "currentTime",
            "operator": "DATE_AFTER",
            "caseInsensitive": false,
            "inverted": false,
            "value": "2025-07-17T23:59:00.000Z"
          },
          {
            "contextName": "remoteAddress",
            "operator": "NUM_GTE",
            "caseInsensitive": false,
            "inverted": false,
            "value": "3333"
          },
          {
            "contextName": "appId",
            "operator": "NUM_EQ",
            "caseInsensitive": false,
            "inverted": false,
            "value": "888"
          },
          {
            "contextName": "appId",
            "operator": "SEMVER_EQ",
            "caseInsensitive": false,
            "inverted": false,
            "value": "1.2.3",
            "values": []
          }
        ]"#;

        let parsed: Vec<Constraint> = serde_json::from_str(data)?;

        assert_eq!(
            parsed,
            vec![
                Constraint {
                    context_name: "appId".to_string(),
                    case_insensitive: false,
                    inverted: false,
                    expression: In {
                        values: vec!["app.known.name".to_string(),],
                    },
                },
                Constraint {
                    context_name: "currentTime".to_string(),
                    case_insensitive: false,
                    inverted: false,
                    expression: DateAfter {
                        value: DateTime::<FixedOffset>::parse_from_rfc3339("2025-07-17T23:59:00Z")
                            .unwrap()
                            .to_utc(),
                    },
                },
                Constraint {
                    context_name: "remoteAddress".to_string(),
                    case_insensitive: false,
                    inverted: false,
                    expression: NumGTE { value: 3333.0 },
                },
                Constraint {
                    context_name: "appId".to_string(),
                    case_insensitive: false,
                    inverted: false,
                    expression: NumEq { value: 888.0 },
                },
                Constraint {
                    context_name: "appId".to_string(),
                    case_insensitive: false,
                    inverted: false,
                    expression: SemverEq {
                        value: Version::from_str("1.2.3").unwrap(),
                    },
                },
            ]
        );
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
