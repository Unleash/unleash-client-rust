// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/api/client/features>
use serde_qs;
use std::collections::HashMap;
use std::default::Default;
use std::fmt::Display;

use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Features {
    pub version: u8,
    pub features: Vec<Feature>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct TagFilter {
    pub name: String,
    pub value: String,
}

impl Display for TagFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.name, self.value)
    }
}

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct FeaturesQuery {
    pub project: Option<String>,
    #[serde(rename = "namePrefix")]
    pub name_prefix: Option<String>,
    #[serde(rename = "tag")]
    pub tags: Option<Vec<String>>,
}

impl FeaturesQuery {
    pub fn new(
        project: Option<String>,
        name_prefix: Option<String>,
        tags: &Option<Vec<TagFilter>>,
    ) -> Self {
        FeaturesQuery {
            project,
            name_prefix,
            tags: tags
                .as_ref()
                .map(|tags| tags.iter().map(ToString::to_string).collect()),
        }
    }
}

impl Features {
    pub fn endpoint(api_url: &str, query: Option<&FeaturesQuery>) -> String {
        let url = format!("{}/client/features", api_url);

        let url = match query {
            Some(query) => format!("{}?{}", url, serde_qs::to_string(query).unwrap()),
            None => url,
        };

        url
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
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
    #[serde(rename = "IN")]
    In(Vec<String>),
    #[serde(rename = "NOT_IN")]
    NotIn(Vec<String>),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "strict", serde(deny_unknown_fields))]
pub struct Variant {
    pub name: String,
    pub weight: u8,
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
    use super::{Features, Registration, TagFilter};
    use crate::api::FeaturesQuery;

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
        let parsed: Features = serde_json::from_str(data)?;
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

    #[test]
    fn test_features_endpoint() {
        let endpoint = Features::endpoint(
            "http://host.example.com:1234/api",
            Some(&FeaturesQuery::new(
                Some("myproject".into()),
                Some("prefix".into()),
                &Some(vec![
                    TagFilter {
                        name: "simple".into(),
                        value: "taga".into(),
                    },
                    TagFilter {
                        name: "simple".into(),
                        value: "tagb".into(),
                    },
                ]),
            )),
        );

        assert_eq!(
            "http://host.example.com:1234/api/client/features?project=myproject&namePrefix=prefix&tag[0]=simple%3Ataga&tag[1]=simple%3Atagb",
            endpoint
        );
    }
}
