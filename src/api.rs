// Copyright 2020 Cognite AS
//! https://unleash.github.io/docs/api/client/features
use std::collections::HashMap;
use std::default::Default;

use chrono::Utc;
use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Feature {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub strategies: Vec<Strategy>,
    pub variants: Option<Vec<Variant>>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Strategy {
    pub name: String,
    pub parameters: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
// #[serde(deny_unknown_fields)]
pub struct Variant {
    pub name: String,
    pub weight: u8,
    pub payload: Option<HashMap<String, String>>,
    pub overrides: Option<Vec<VariantOverride>>,
}

#[derive(Serialize, Deserialize, Debug)]
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
