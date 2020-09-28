// Copyright 2020 Cognite AS

//!
//! Set environment variables as per config.rs to run this.
//! It will query a feature called "default", report status for it, and upload a
//! metric bucket.

use std::time::Duration;

use async_std::task;
use enum_map::Enum;
use futures_timer::Delay;
use serde::{Deserialize, Serialize};

use unleash_api_client::client;
use unleash_api_client::config::EnvironmentConfig;

#[allow(non_camel_case_types)]
#[derive(Debug, Deserialize, Serialize, Enum, Clone)]
enum UserFeatures {
    default,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let _ = simple_logger::SimpleLogger::new().init();
    task::block_on(async {
        let config = EnvironmentConfig::from_env()?;
        let client = client::ClientBuilder::default()
            .interval(500)
            .into_client::<UserFeatures>(
                &config.api_url,
                &config.app_name,
                &config.instance_id,
                config.secret,
            )?;
        client.register().await?;
        futures::future::join(client.poll_for_updates(), async {
            // Ensure we have features for this demo.
            Delay::new(Duration::from_millis(500)).await;
            println!(
                "feature 'default' is {}",
                client.is_enabled(UserFeatures::default, None, false)
            );
            // Wait to allow metrics upload
            Delay::new(Duration::from_millis(500)).await;
            // allow the future::join to finish
            client.stop_poll().await;
        })
        .await;
        Ok(())
    })
}
