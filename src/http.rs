// Copyright 2020, 2022 Cognite AS
//! The HTTP Layer

#[cfg(feature = "reqwest")]
mod reqwest;
#[cfg(feature = "reqwest-11")]
mod reqwest_11;
mod shim;
#[cfg(feature = "surf")]
mod surf;

pub struct HTTP<C: HttpClient> {
    authorization_header: C::HeaderName,
    app_name_header: C::HeaderName,
    unleash_app_name_header: C::HeaderName,
    unleash_sdk_header: C::HeaderName,
    unleash_connection_id_header: C::HeaderName,
    instance_id_header: C::HeaderName,
    app_name: String,
    sdk_version: &'static str,
    instance_id: String,
    // The connection_id represents a logical connection from the SDK to Unleash.
    // It's assigned internally by the SDK and lives as long as the Unleash client instance.
    // We can't reuse instance_id since some SDKs allow to override it while
    // connection_id has to be uniquely defined by the SDK.
    connection_id: String,
    authorization: Option<String>,
    client: C,
}

use crate::version::get_sdk_version;
use serde::{de::DeserializeOwned, Serialize};
#[doc(inline)]
pub use shim::HttpClient;

impl<C> HTTP<C>
where
    C: HttpClient + Default,
{
    /// The error type on this will change in future.
    pub fn new(
        app_name: String,
        instance_id: String,
        connection_id: String,
        authorization: Option<String>,
    ) -> Result<Self, C::Error> {
        Ok(HTTP {
            client: C::default(),
            app_name,
            sdk_version: get_sdk_version(),
            connection_id,
            instance_id,
            authorization,
            authorization_header: C::build_header("authorization")?,
            app_name_header: C::build_header("appname")?,
            unleash_app_name_header: C::build_header("unleash-appname")?,
            unleash_sdk_header: C::build_header("unleash-sdk")?,
            unleash_connection_id_header: C::build_header("unleash-connection-id")?,
            instance_id_header: C::build_header("instance_id")?,
        })
    }

    /// Perform a GET. Returns errors per HttpClient::get.
    pub fn get(&self, uri: &str) -> C::RequestBuilder {
        let request = self.client.get(uri);
        self.attach_headers(request)
    }

    /// Make a get request and parse into JSON
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        interval: Option<u64>,
    ) -> Result<T, C::Error> {
        let mut request = self.get(endpoint);
        if let Some(interval) = interval {
            request = C::header(
                request,
                &C::build_header("unleash-interval")?,
                &interval.to_string(),
            );
        }
        C::get_json(request).await
    }

    /// Perform a POST. Returns errors per HttpClient::post.
    pub fn post(&self, uri: &str) -> C::RequestBuilder {
        let request = self.client.post(uri);
        self.attach_headers(request)
    }

    /// Encode content into JSON and post to an endpoint. Returns the statuscode
    /// is_success() value.
    pub async fn post_json<T: Serialize + Sync>(
        &self,
        endpoint: &str,
        content: T,
        interval: Option<u64>,
    ) -> Result<bool, C::Error> {
        let mut request = self.post(endpoint);
        if let Some(interval) = interval {
            request = C::header(
                request,
                &C::build_header("unleash-interval")?,
                &interval.to_string(),
            );
        }
        C::post_json(request, &content).await
    }

    fn attach_headers(&self, request: C::RequestBuilder) -> C::RequestBuilder {
        let request = C::header(request, &self.app_name_header, self.app_name.as_str());
        let request = C::header(
            request,
            &self.unleash_app_name_header,
            self.app_name.as_str(),
        );
        let request = C::header(request, &self.unleash_sdk_header, self.sdk_version);
        let request = C::header(
            request,
            &self.unleash_connection_id_header,
            self.connection_id.as_str(),
        );
        let request = C::header(request, &self.instance_id_header, self.instance_id.as_str());
        if let Some(auth) = &self.authorization {
            C::header(request, &self.authorization_header.clone(), auth.as_str())
        } else {
            request
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use regex::Regex;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use uuid::Uuid;

    #[derive(Clone, Default)]
    struct MockHttpClient {
        headers: Arc<RwLock<HashMap<String, String>>>,
    }

    #[async_trait]
    impl HttpClient for MockHttpClient {
        type Error = std::io::Error;
        type HeaderName = String;
        type RequestBuilder = Self;

        fn build_header(name: &'static str) -> Result<Self::HeaderName, Self::Error> {
            Ok(name.to_string())
        }

        fn header(builder: Self, key: &Self::HeaderName, value: &str) -> Self::RequestBuilder {
            if let Ok(mut headers) = builder.headers.write() {
                headers.insert(key.clone(), value.to_string());
            }
            builder
        }

        fn get(&self, _uri: &str) -> Self::RequestBuilder {
            self.clone()
        }

        fn post(&self, _uri: &str) -> Self::RequestBuilder {
            self.clone()
        }

        async fn get_json<T: DeserializeOwned>(
            _req: Self::RequestBuilder,
        ) -> Result<T, Self::Error> {
            Ok(serde_json::from_value(json!({})).unwrap())
        }

        async fn post_json<T: Serialize + Sync>(
            _req: Self::RequestBuilder,
            _content: &T,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn test_specific_headers() {
        let http_client = HTTP::<MockHttpClient>::new(
            "my_app".to_string(),
            "my_instance_id".to_string(),
            "d512f8ec-d972-40a5-9a30-a0a6e85d93ac".to_string(),
            Some("auth_token".to_string()),
        )
        .unwrap();

        let _ = http_client
            .get_json::<serde_json::Value>("http://example.com", Some(15))
            .await;
        let headers = &http_client.client.headers.read().unwrap();

        assert_eq!(headers.get("unleash-appname").unwrap(), "my_app");
        assert_eq!(headers.get("instance_id").unwrap(), "my_instance_id");
        assert_eq!(
            headers.get("unleash-connection-id").unwrap(),
            "d512f8ec-d972-40a5-9a30-a0a6e85d93ac"
        );
        assert_eq!(headers.get("unleash-interval").unwrap(), "15");
        assert_eq!(headers.get("authorization").unwrap(), "auth_token");

        let version_regex = Regex::new(r"^unleash-client-rust:\d+\.\d+\.\d+$").unwrap();
        let sdk_version = headers.get("unleash-sdk").unwrap();
        assert!(
            version_regex.is_match(sdk_version),
            "Version output did not match expected format: {}",
            sdk_version
        );

        let connection_id = headers.get("unleash-connection-id").unwrap();
        assert!(
            Uuid::parse_str(connection_id).is_ok(),
            "Connection ID is not a valid UUID"
        );
    }
}
