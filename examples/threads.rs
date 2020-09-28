// Copyright 2020 Cognite AS

//!
//! Set environment variables as per config.rs to run this.
//! It will query a feature called "default", report status for it, and upload a
//! metric bucket.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use async_std::task;
use enum_map::Enum;
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
    let config = EnvironmentConfig::from_env()?;
    let client = Arc::new(
        client::ClientBuilder::default()
            .interval(500)
            .into_client::<UserFeatures>(
                &config.api_url,
                &config.app_name,
                &config.instance_id,
                config.secret,
            )?,
    );
    task::block_on(async {
        if let Err(e) = client.register().await {
            return Err(e);
        } else {
            Ok(())
        }
    })?;
    // Spin off a polling thread
    let poll_handle = client.clone();
    // let poll_handle = think.clone();
    let handler = thread::spawn(move || {
        // thread code
        task::block_on(async {
            poll_handle.poll_for_updates().await;
        });
    });

    // Ensure we have features for this demo.
    thread::sleep(Duration::from_millis(500));
    println!(
        "feature 'default' is {}",
        client.is_enabled(UserFeatures::default, None, false)
    );
    // Wait to allow metrics upload
    thread::sleep(Duration::from_millis(500));
    // allow the thread handler.join() to finish
    task::block_on(async {
        client.stop_poll().await;
    });
    handler.join().unwrap();
    Ok(())
}
