//! Shim reqwest into an unleash HTTP client

// Copyright 2022 Cognite AS

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

use super::HttpClient;

#[async_trait]
impl HttpClient for reqwest_11::Client {
    type HeaderName = reqwest_11::header::HeaderName;
    type Error = reqwest_11::Error;
    type RequestBuilder = reqwest_11::RequestBuilder;

    fn build_header(name: &'static str) -> Result<Self::HeaderName, Self::Error> {
        Ok(Self::HeaderName::from_static(name))
    }

    fn get(&self, uri: &str) -> Self::RequestBuilder {
        self.get(uri)
    }

    fn post(&self, uri: &str) -> Self::RequestBuilder {
        self.post(uri)
    }

    fn header(
        builder: Self::RequestBuilder,
        key: &Self::HeaderName,
        value: &str,
    ) -> Self::RequestBuilder {
        builder.header(key.clone(), value)
    }

    async fn get_json<T: DeserializeOwned>(req: Self::RequestBuilder) -> Result<T, Self::Error> {
        req.send().await?.json::<T>().await
    }

    async fn post_json<T: Serialize + Sync>(
        req: Self::RequestBuilder,
        content: &T,
    ) -> Result<bool, Self::Error> {
        let req = req.json(content);
        let res = req.send().await?;
        Ok(res.status().is_success())
    }
}
