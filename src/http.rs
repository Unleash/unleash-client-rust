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
    connection_id: String,
    authorization: Option<String>,
    client: C,
}

use crate::version::get_sdk_version;
use serde::{de::DeserializeOwned, Serialize};
#[doc(inline)]
pub use shim::HttpClient;
use uuid::Uuid;

impl<C> HTTP<C>
where
    C: HttpClient + Default,
{
    /// The error type on this will change in future.
    pub fn new(
        app_name: String,
        instance_id: String,
        authorization: Option<String>,
    ) -> Result<Self, C::Error> {
        Ok(HTTP {
            client: C::default(),
            app_name,
            sdk_version: get_sdk_version(),
            connection_id: Uuid::new_v4().to_string(),
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
    pub async fn get_json<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, C::Error> {
        C::get_json(self.get(endpoint)).await
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
    ) -> Result<bool, C::Error> {
        C::post_json(self.post(endpoint), &content).await
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

    #[derive(Clone, Default)]
    struct MockHttpClient {
        headers: std::collections::HashMap<String, String>,
    }

    #[async_trait]
    impl HttpClient for MockHttpClient {
        type Error = std::io::Error;
        type HeaderName = String;
        type RequestBuilder = Self;

        fn build_header(name: &'static str) -> Result<Self::HeaderName, Self::Error> {
            Ok(name.to_string())
        }

        fn header(mut builder: Self, key: &Self::HeaderName, value: &str) -> Self::RequestBuilder {
            builder.headers.insert(key.clone(), value.to_string());
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
            unimplemented!()
        }

        async fn post_json<T: Serialize + Sync>(
            _req: Self::RequestBuilder,
            _content: &T,
        ) -> Result<bool, Self::Error> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn test_specific_headers() {
        let http_client = HTTP::<MockHttpClient>::new(
            "my_app".to_string(),
            "my_instance_id".to_string(),
            Some("auth_token".to_string()),
        )
        .unwrap();

        let request_builder = http_client.client.post("http://example.com");
        let request_with_headers = http_client.attach_headers(request_builder);

        assert_eq!(
            request_with_headers.headers.get("unleash-appname").unwrap(),
            "my_app"
        );
        assert_eq!(
            request_with_headers.headers.get("instance_id").unwrap(),
            "my_instance_id"
        );
        assert_eq!(
            request_with_headers.headers.get("authorization").unwrap(),
            "auth_token"
        );

        let version_regex = Regex::new(r"^unleash-client-rust:\d+\.\d+\.\d+$").unwrap();
        let sdk_version = request_with_headers.headers.get("unleash-sdk").unwrap();
        assert!(
            version_regex.is_match(sdk_version),
            "Version output did not match expected format: {}",
            sdk_version
        );

        let connection_id = request_with_headers
            .headers
            .get("unleash-connection-id")
            .unwrap();
        assert!(
            Uuid::parse_str(connection_id).is_ok(),
            "Connection ID is not a valid UUID"
        );
    }
}
