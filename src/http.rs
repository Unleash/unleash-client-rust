// Copyright 2020 Cognite AS
//! The HTTP Layer

#[cfg(feature = "surf")]
use surf::{http::headers::HeaderName, Client};
#[cfg(feature = "surf")]
pub use surf::{Error, RequestBuilder, Response};

#[cfg(feature = "reqwest")]
use reqwest::{header::HeaderName, Client};
#[cfg(feature = "reqwest")]
pub use reqwest::{Error, RequestBuilder, Response};

pub struct HTTP {
    authorization_header: HeaderName,
    app_name_header: HeaderName,
    instance_id_header: HeaderName,
    app_name: String,
    instance_id: String,
    authorization: Option<String>,
    client: Client,
}

impl HTTP {
    /// The error type on this will change in future.
    pub fn new(
        app_name: String,
        instance_id: String,
        authorization: Option<String>,
    ) -> Result<Self, Error> {
        #[cfg(feature = "surf")]
        fn build_header(name: &'static str) -> Result<HeaderName, Error> {
            HeaderName::from_bytes(name.into())
        }

        #[cfg(feature = "reqwest")]
        fn build_header(name: &'static str) -> Result<HeaderName, Error> {
            Ok(HeaderName::from_static(name))
        }

        Ok(HTTP {
            client: Client::new(),
            app_name,
            instance_id,
            authorization,
            authorization_header: build_header("authorization")?,
            app_name_header: build_header("appname")?,
            instance_id_header: build_header("instance_id")?,
        })
    }

    /// Perform a GET. Returns errors per surf::Client::get.
    pub fn get(&self, uri: impl AsRef<str>) -> RequestBuilder {
        let request = self
            .client
            .get(uri.as_ref())
            .header(self.app_name_header.clone(), self.app_name.as_str())
            .header(self.instance_id_header.clone(), self.instance_id.as_str());
        if let Some(auth) = &self.authorization {
            request.header(self.authorization_header.clone(), auth.as_str())
        } else {
            request
        }
    }

    /// Perform a POST. Returns errors per surf::Client::get.
    pub fn post(&self, uri: impl AsRef<str>) -> RequestBuilder {
        let request = self
            .client
            .post(uri.as_ref())
            .header(self.app_name_header.clone(), self.app_name.as_str())
            .header(self.instance_id_header.clone(), self.instance_id.as_str());
        if let Some(auth) = &self.authorization {
            request.header(self.authorization_header.clone(), auth.as_str())
        } else {
            request
        }
    }
}
