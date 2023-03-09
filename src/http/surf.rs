//! Shim surf into an unleash HTTP client

// Copyright 2022 Cognite AS

use std::{error::Error, fmt::Display};

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

use super::HttpClient;

#[async_trait]
impl HttpClient for surf::Client {
    type HeaderName = surf::http::headers::HeaderName;
    type Error = SurfStdError;
    type RequestBuilder = surf::RequestBuilder;

    fn build_header(name: &'static str) -> Result<Self::HeaderName, Self::Error> {
        Self::HeaderName::from_bytes(name.into()).map_err(SurfStdError)
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
        req.recv_json::<T>().await.map_err(SurfStdError)
    }

    async fn post_json<T: Serialize + Sync>(
        req: Self::RequestBuilder,
        content: &T,
    ) -> Result<bool, Self::Error> {
        async {
            let req = req.body_json(content)?;
            let res = req.await?;
            Ok(res.status().is_success())
        }
        .await
        .map_err(SurfStdError)
    }
}

#[derive(Debug)]
pub struct SurfStdError(surf::Error);

impl Display for SurfStdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for SurfStdError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        AsRef::<dyn Error>::as_ref(&self.0).source()
        // This would be ideal, but surf::Error doesn't implement surf::Error::Iterator
        // for cause in self.0.chain() {
        //     return cause;
        // }

        // None
    }

    #[rustversion::any(nightly)]
    #[cfg(feature = "backtrace")]
    fn backtrace(&self) -> Option<&std::backtrace::Backtrace> {
        self.0.backtrace()
    }
}
