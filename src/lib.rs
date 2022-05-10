// Copyright 2020,2021 Cognite AS
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
//! you will need to arrange to call the `poll_for_updates` method from a thread as
//! demonstrated in `examples/threads.rs`.
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
//! use serde::{Deserialize, Serialize};
//! use enum_map::Enum;
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
//! #[allow(non_camel_case_types)]
//! #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
//! enum UserFeatures {
//!     default
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
//!     let _ = simple_logger::init();
//!     task::block_on(async {
//!         let config = EnvironmentConfig::from_env()?;
//!         let client = client::ClientBuilder::default()
//!             .strategy("reversed", Box::new(&_reversed_uids))
//!             .into_client::<UserFeatures>(
//!                 &config.api_url,
//!                 &config.app_name,
//!                 &config.instance_id,
//!                config.secret,
//!             )?;
//!         client.register().await?;
//!         futures::future::join(client.poll_for_updates(), async {
//!             // Ensure we have initial load of features completed
//!             Delay::new(Duration::from_millis(500)).await;
//!             assert_eq!(true, client.is_enabled(UserFeatures::default, None, false));
//!             // ... serve more requests
//!             client.stop_poll().await;
//!         }).await;
//!         Ok(())
//!     })
//! }
//! ```
//!
//!  Previously there was a Re-export of enum_map::Enum - this trait is part of
//!  our public API. But there is a bug:
//!  https://gitlab.com/KonradBorowski/enum-map/-/issues/22 so instead you must
//!  match the version in your dependencies.
#![warn(clippy::all)]

#[cfg(not(any(
    feature = "surf-client",
    feature = "reqwest-client",
    feature = "reqwest-client-rustls"
)))]
compile_error!("No HTTP client configured, enable one of the *-client features.");

pub mod api;
pub mod client;
pub mod config;
pub mod context;
pub mod http;
pub mod strategy;

// Exports for ergonomical use
pub use crate::client::{Client, ClientBuilder};
pub use crate::config::EnvironmentConfig;
pub use crate::context::Context;
pub use crate::strategy::Evaluate;

/// For the complete minimalist
///
/// ```no_run
/// use serde::{Deserialize, Serialize};
/// use enum_map::Enum;
/// use unleash_api_client::prelude::*;
///
/// let config = EnvironmentConfig::from_env()?;
///
/// #[allow(non_camel_case_types)]
/// #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
/// enum UserFeatures {
///     feature
/// }
///
/// let client = ClientBuilder::default()
///     .into_client::<UserFeatures>(
///         &config.api_url,
///         &config.app_name,
///         &config.instance_id,
///         config.secret,
///         )?;
/// # Ok::<(), Box<dyn std::error::Error + std::marker::Send + std::marker::Sync>>(())
/// ```
pub mod prelude {
    pub use crate::client::ClientBuilder;
    pub use crate::config::EnvironmentConfig;
}
