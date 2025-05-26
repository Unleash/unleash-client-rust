// Copyright 2020 Cognite AS

//!
//! Set environment variables as per config.rs to run this.
//! It will query a feature called "default", report status for it, and upload a
//! metric bucket.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use enum_map::Enum;
use serde::{Deserialize, Serialize};

use unleash_api_client::{client, config::EnvironmentConfig};

#[allow(non_camel_case_types)]
#[derive(Debug, Deserialize, Serialize, Enum, Clone)]
enum UserFeatures {
    default,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "surf")] {
            use core::future::Future;
            use surf::Client as HttpClient;
            use async_std::task;
            #[derive(Clone)]
            struct RT {}
            impl RT {
                fn block_on<F: Future>(&self, future: F) -> F::Output {
                    task::block_on(future)
                }
            }
            let rt = RT{};
        } else if #[cfg(feature = "reqwest")] {
            use reqwest::Client as HttpClient;
            use tokio::runtime::Runtime;
            let rt = Arc::new(Runtime::new().unwrap());
        } else if #[cfg(feature = "reqwest-11")] {
            use reqwest_11::Client as HttpClient;
            use tokio::runtime::Runtime;
            let rt = Arc::new(Runtime::new().unwrap());
        } else {
            compile_error!("Cannot run test suite without a client enabled");
        }
    }

    let _ = simple_logger::SimpleLogger::new()
        .with_utc_timestamps()
        .init();
    let config = EnvironmentConfig::from_env()?;
    let client = Arc::new(
        client::ClientBuilder::default()
            .interval(500)
            .into_client::<UserFeatures, HttpClient>(
                &config.api_url,
                &config.app_name,
                &config.instance_id,
                config.secret,
            )?,
    );
    // remove when https://github.com/rust-lang/rust/issues/102616 is fixed
    #[allow(clippy::question_mark)]
    rt.block_on(async {
        if let Err(e) = client.register().await {
            Err(e)
        } else {
            Ok(())
        }
    })?;
    // Spin off a polling thread
    let poll_handle = client.clone();
    // let poll_handle = think.clone();
    let inner_rt = rt.clone();
    let handler = thread::spawn(move || {
        // thread code
        inner_rt.block_on(async {
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
    rt.block_on(async {
        client.stop_poll().await;
    });
    handler.join().unwrap();
    Ok(())
}
