// Copyright 2020 Cognite AS

//! Runs the Unleash provided client conformance tests.

mod tests {
    use std::env;
    use std::fs;

    use enum_map::Enum;
    use serde::{Deserialize, Serialize};

    use unleash_api_client::{api, client, context};

    #[derive(Debug, Deserialize)]
    struct Test {
        description: String,
        context: context::Context,
        #[serde(rename = "toggleName")]
        toggle_name: String,
        #[serde(rename = "expectedResult")]
        expected_result: bool,
    }

    #[derive(Debug, Deserialize)]
    struct Payload {
        #[serde(rename = "type")]
        _type: String,
        #[serde(rename = "value")]
        _value: String,
    }

    #[derive(Debug, Deserialize)]
    struct VariantResult {
        #[serde(rename = "name")]
        _name: String,
        #[serde(rename = "payload")]
        _payload: Option<Payload>,
        enabled: bool,
    }

    impl PartialEq<client::Variant> for VariantResult {
        fn eq(&self, other: &client::Variant) -> bool {
            let payload_matches = match &self._payload {
                Some(payload) => match (other.payload.get("type"), other.payload.get("value")) {
                    (Some(_type), Some(value)) => {
                        &payload._type == _type && &payload._value == value
                    }
                    _ => false,
                },
                None => !other.payload.contains_key("type") && !other.payload.contains_key("value"),
            };
            self.enabled == other.enabled && self._name == other.name && payload_matches
        }
    }

    #[derive(Debug, Deserialize)]
    struct VariantTest {
        description: String,
        context: context::Context,
        #[serde(rename = "toggleName")]
        toggle_name: String,
        #[serde(rename = "expectedResult")]
        expected_result: VariantResult,
    }

    #[derive(Debug, Deserialize)]
    enum Tests {
        #[serde(rename = "tests")]
        Tests(Vec<Test>),
        #[serde(rename = "variantTests")]
        VariantTests(Vec<VariantTest>),
    }

    #[derive(Debug, Deserialize)]
    struct Suite {
        #[serde(rename = "name")]
        _name: String,
        state: api::Features,
        #[serde(flatten)]
        tests: Tests,
    }

    #[test]
    fn test_client_specification() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
    {
        cfg_if::cfg_if! {
            if #[cfg(feature = "reqwest")] {
                use reqwest::Client as HttpClient;
            } else if #[cfg(feature = "reqwest-11")] {
                use reqwest_11::Client as HttpClient;
            } else {
                compile_error!("Cannot run test suite without a client enabled");
            }
        }
        let _ = simple_logger::SimpleLogger::new()
            .with_utc_timestamps()
            .with_module_level("isahc::agent", log::LevelFilter::Off)
            .with_module_level("tracing::span", log::LevelFilter::Off)
            .with_module_level("tracing::span::active", log::LevelFilter::Off)
            .init();
        let current_exe_path = env::current_exe().unwrap();
        let mut exe_dir = current_exe_path.parent().unwrap();
        if exe_dir.ends_with("deps") {
            exe_dir = exe_dir.parent().unwrap();
        }
        let spec_dir = exe_dir.join("../../client-specification/specifications/");

        log::info!("Loading tests from {}", spec_dir.display());
        let index = fs::read(spec_dir.join("index.json"))?;
        let suite_names: Vec<String> = serde_json::from_slice(&index)?;
        for suite_name in suite_names {
            log::info!("Running suite {suite_name}");
            let suite_content = fs::read(spec_dir.join(&suite_name))?;
            let suite: Suite = serde_json::from_slice(&suite_content)?;

            #[allow(non_camel_case_types)]
            #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
            enum NoFeatures {}
            let c = client::ClientBuilder::default()
                .enable_string_features()
                .into_client::<NoFeatures, HttpClient>(
                    "http://127.0.0.1:1234/",
                    "foo",
                    "test",
                    None,
                )
                .unwrap();
            log::info!("Using features {:?}", &suite.state.features);
            c.memoize(suite.state.features).unwrap();

            match suite.tests {
                Tests::Tests(tests) => {
                    for test in tests {
                        assert_eq!(
                            test.expected_result,
                            c.is_enabled_str(&test.toggle_name, Some(&test.context), false),
                            "Test '{}' in suite '{}' failed: got {} instead of {}",
                            test.description,
                            suite_name,
                            !test.expected_result,
                            test.expected_result
                        );
                    }
                }
                Tests::VariantTests(variant_tests) => {
                    for test in variant_tests {
                        let result = c.get_variant_str(&test.toggle_name, &test.context);

                        assert_eq!(
                            test.expected_result, result,
                            "Test '{}' in suite '{}' failed: got {:?} instead of {:?}",
                            test.description, suite_name, result, test.expected_result
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
