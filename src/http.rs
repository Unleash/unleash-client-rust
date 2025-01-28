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
    x_app_name_header: C::HeaderName,
    x_sdk_header: C::HeaderName,
    x_connection_id_header: C::HeaderName,
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
            x_app_name_header: C::build_header("x-unleash-appname")?,
            x_sdk_header: C::build_header("x-unleash-sdk")?,
            x_connection_id_header: C::build_header("x-unleash-connection-id")?,
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
        let request = C::header(request, &self.x_app_name_header, self.app_name.as_str());
        let request = C::header(request, &self.x_sdk_header, self.sdk_version);
        let request = C::header(
            request,
            &self.x_connection_id_header,
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
