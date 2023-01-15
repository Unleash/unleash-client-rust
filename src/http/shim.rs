//! Shim traits to define an external client as an unleash HTTP client This is
//! an imperfect system, but sufficient to work with multiple async frameworks,
//! which is the goal.

// Copyright 2022 Cognite AS

use core::fmt::{Debug, Display};
use std::error::Error;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

/// Abstraction over the concrete HTTP client being used. Implement this on any
/// type to use it as an HTTP client.
#[async_trait]
pub trait HttpClient: Sync + Send {
    type HeaderName: Clone + Sync + Send;
    type Error: Debug + Display + Error + Send + Sync + 'static;
    type RequestBuilder;

    /// Construct a HTTP client layer headername
    fn build_header(name: &'static str) -> Result<Self::HeaderName, Self::Error>;

    /// Make a get request
    fn get(&self, uri: &str) -> Self::RequestBuilder;

    /// Make a post  request
    fn post(&self, uri: &str) -> Self::RequestBuilder;

    /// Add a header to a request
    fn header(
        builder: Self::RequestBuilder,
        key: &Self::HeaderName,
        value: &str,
    ) -> Self::RequestBuilder;

    /// Add a query to a request
    fn query(
        builder: Self::RequestBuilder,
        query: &impl Serialize,
    ) -> Result<Self::RequestBuilder, Self::Error>;

    /// Make a get request and parse into JSON
    async fn get_json<T: DeserializeOwned>(req: Self::RequestBuilder) -> Result<T, Self::Error>;

    /// Encode content into JSON and post to an endpoint. Returns the statuscode
    /// is_success() value.
    async fn post_json<T: Serialize + Sync>(
        req: Self::RequestBuilder,
        content: &T,
    ) -> Result<bool, Self::Error>;
}
