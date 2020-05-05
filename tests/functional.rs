// Copyright 2020 Cognite AS

//! Functional test against an unleashed API server running locally.
//! Set environment variables as per config.rs to exercise this.
use async_std::task;

use unleash_api_client::api;
use unleash_api_client::client;
use unleash_api_client::config::EnvironmentConfig;

#[test]
fn test_register() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    task::block_on(async {
        let config = EnvironmentConfig::from_env()?;
        let client = client::Client::<http_client::native::NativeClient>::new(
            &config.api_url,
            &config.app_name,
            &config.instance_id,
            config.secret,
        )?;
        client.register().await?;
        Ok(())
    })
}
