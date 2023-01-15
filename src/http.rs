// Copyright 2020, 2022 Cognite AS
//! The HTTP Layer

#[cfg(feature = "reqwest")]
mod reqwest;
mod shim;
#[cfg(feature = "surf")]
mod surf;

pub struct HTTP<C: HttpClient> {
    authorization_header: C::HeaderName,
    app_name_header: C::HeaderName,
    instance_id_header: C::HeaderName,
    app_name: String,
    instance_id: String,
    authorization: Option<String>,
    client: C,
}

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
        authorization: Option<String>,
    ) -> Result<Self, C::Error> {
        Ok(HTTP {
            client: C::default(),
            app_name,
            instance_id,
            authorization,
            authorization_header: C::build_header("authorization")?,
            app_name_header: C::build_header("appname")?,
            instance_id_header: C::build_header("instance_id")?,
        })
    }

    /// Perform a GET. Returns errors per HttpClient::get.
    pub fn get(
        &self,
        uri: &str,
        query: Option<&impl Serialize>,
    ) -> Result<C::RequestBuilder, C::Error> {
        let request = self.client.get(uri);

        let request = match query {
            Some(query) => C::query(request, query)?,
            None => request,
        };

        Ok(self.attach_headers(request))
    }

    /// Make a get request and parse into JSON
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        query: Option<&impl Serialize>,
    ) -> Result<T, C::Error> {
        C::get_json(self.get(endpoint, query)?).await
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
        let request = C::header(request, &self.instance_id_header, self.instance_id.as_str());
        if let Some(auth) = &self.authorization {
            C::header(request, &self.authorization_header.clone(), auth.as_str())
        } else {
            request
        }
    }
}
