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
    #[serde(untagged)]
    enum Tests {
        Tests {
            tests: Vec<Test>,
        },
        VariantTests {
            #[serde(rename = "variantTests")]
            variant_tests: Vec<VariantTest>,
        },
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
        let _ = simple_logger::SimpleLogger::new()
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
            log::info!("Running suite {}", suite_name);
            let suite_content = fs::read(spec_dir.join(suite_name))?;
            let suite: Suite = serde_json::from_slice(&suite_content)?;

            assert_eq!(1, suite.state.version);

            #[allow(non_camel_case_types)]
            #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
            enum NoFeatures {}
            let c = client::ClientBuilder::default()
                .enable_string_features()
                .into_client::<NoFeatures>("http://127.0.0.1:1234/", "foo", "test", None)
                .unwrap();
            log::info!("Using features {:?}", &suite.state.features);
            c.memoize(suite.state.features).unwrap();

            match suite.tests {
                Tests::Tests { tests } => {
                    for test in tests {
                        assert_eq!(
                            test.expected_result,
                            c.is_enabled_str(&test.toggle_name, Some(&test.context), false),
                            "Test '{}' failed: got {} instead of {}",
                            test.description,
                            !test.expected_result,
                            test.expected_result
                        );
                    }
                }
                Tests::VariantTests { variant_tests } => {
                    for test in variant_tests {
                        let result =
                            c.is_enabled_str(&test.toggle_name, Some(&test.context), false);
                        assert_eq!(
                            test.expected_result.enabled, result,
                            "Test '{}' failed: got {} instead of {:?}",
                            test.description, result, test.expected_result
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
