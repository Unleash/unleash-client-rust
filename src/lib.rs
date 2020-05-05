// Copyright 2020 Cognite AS
//! [Unleash](https://unleash.github.io) is a feature flag API system. This is a
//! client for it to facilitate using the API to control features in Rust programs.
//!
//! ## Client overview
//!
//! The client is written using async. To use it in a sync program, run an async
//! executor and `block_on()` the relevant calls. As the client specification
//! requires sending background metrics to the API, you will need to arrange to
//! call the `submit_metrics` method periodically.

#![warn(clippy::all)]
pub mod api;
pub mod client;
pub mod config;
pub mod context;
pub mod http;
pub mod strategy;
