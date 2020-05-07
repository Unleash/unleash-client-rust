// Copyright 2020 Cognite AS
//! [Unleash](https://unleash.github.io) is a feature flag API system. This is a
//! client for it to facilitate using the API to control features in Rust programs.
//!
//! ## Client overview
//!
//! The client is written using async. Any std compatible async runtime should be
//! compatible. Examples with async-std and tokio are in the examples/ in the source
//! tree.
//!
//! To use it in a sync program, run an async executor and `block_on()` the relevant
//! calls. As the client specification requires sending background metrics to the API,
//! you will need to arrange to call the `poll` method from a thread. Contributions
//! to provide helpers to make this easier are welcome.
//!
//! The unleash defined strategies are included, to support custom strategies
//! use the `ClientBuilder` and call the `strategy` method to register your custom
//! strategy memoization function.
//!
//! ```no_run
//! use std::collections::hash_map::HashMap;
//! use std::collections::hash_set::HashSet;
//! use std::hash::BuildHasher;
//! use std::time::Duration;
//!
//! use async_std::task;
//! use futures_timer::Delay;
//!
//! use unleash_api_client::client;
//! use unleash_api_client::config::EnvironmentConfig;
//! use unleash_api_client::context::Context;
//! use unleash_api_client::strategy;
//!
//! fn _reversed_uids<S: BuildHasher>(
//!     parameters: Option<HashMap<String, String, S>>,
//! ) -> strategy::Evaluate {
//!     let mut uids: HashSet<String> = HashSet::new();
//!     if let Some(parameters) = parameters {
//!         if let Some(uids_list) = parameters.get("userIds") {
//!             for uid in uids_list.split(',') {
//!                 uids.insert(uid.chars().rev().collect());
//!             }
//!         }
//!     }
//!     Box::new(move |context: &Context| -> bool {
//!         context
//!             .user_id
//!             .as_ref()
//!             .map(|uid| uids.contains(uid))
//!             .unwrap_or(false)
//!     })
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
//!     let _ = simple_logger::init();
//!     task::block_on(async {
//!         let config = EnvironmentConfig::from_env()?;
//!         let client = client::ClientBuilder::default()
//!             .strategy("reversed", Box::new(&_reversed_uids))
//!             .into_client::<http_client::native::NativeClient>(
//!                 &config.api_url,
//!                 &config.app_name,
//!                 &config.instance_id,
//!                config.secret,
//!             )?;
//!         client.register().await?;
//!         futures::future::join(client.poll(), async {
//!             // Ensure we have initial load of features completed
//!             Delay::new(Duration::from_millis(500)).await;
//!             assert_eq!(true, client.is_enabled("default", None, false));
//!             // ... serve more requests
//!             client.stop_poll().await;
//!         }).await;
//!         Ok(())
//!     })
//! }
//! ```

#![warn(clippy::all)]
pub mod api;
pub mod client;
pub mod config;
pub mod context;
pub mod http;
pub mod strategy;
