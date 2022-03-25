// Copyright 2020 Cognite AS

//! Convenient configuration glue.
//!
//! This module grabs configuration settings from the environment in an
//! easy-to-hand-off fashion. It is loosely coupled so that apps using
//! different configuration mechanisms are not required to use it or even
//! implement a trait.

use std::env;

#[derive(Debug, Default)]
pub struct EnvironmentConfig {
    pub api_url: String,
    pub app_name: String,
    pub instance_id: String,
    pub secret: Option<String>,
}

impl EnvironmentConfig {
    /// Retrieve a configuration from environment variables.alloc
    ///
    /// UNLEASH_API_URL: <http://host.example.com:1234/api>
    /// UNLEASH_APP_NAME: example-app
    /// UNLEASH_INSTANCE_ID: instance-512
    /// UNLEASH_CLIENT_SECRET: unset | some-secret-value
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut result = EnvironmentConfig::default();
        let api_url = env::var("UNLEASH_API_URL");
        if let Ok(api_url) = api_url {
            result.api_url = api_url;
        } else {
            return Err(anyhow::anyhow!("UNLEASH_API_URL not set").into());
        };
        if let Ok(app_name) = env::var("UNLEASH_APP_NAME") {
            result.app_name = app_name;
        } else {
            return Err(anyhow::anyhow!("UNLEASH_APP_NAME not set").into());
        };
        if let Ok(instance_id) = env::var("UNLEASH_INSTANCE_ID") {
            result.instance_id = instance_id;
        } else {
            return Err(anyhow::anyhow!("UNLEASH_INSTANCE_ID not set").into());
        };
        result.secret = env::var("UNLEASH_CLIENT_SECRET").ok();
        Ok(result)
    }
}
