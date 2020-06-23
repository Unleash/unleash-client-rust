// Copyright 2020 Cognite AS
//! The HTTP Layer
use std::default::Default;

use http_types::headers;

pub struct HTTP<C: http_client::HttpClient> {
    authorization_header: headers::HeaderName,
    app_name_header: headers::HeaderName,
    instance_id_header: headers::HeaderName,
    app_name: String,
    instance_id: String,
    authorization: Option<String>,
    client: surf::Client<C>,
}

impl<C: http_client::HttpClient + std::default::Default> HTTP<C> {
    /// The error type on this will change in future.
    pub fn new(
        app_name: String,
        instance_id: String,
        authorization: Option<String>,
    ) -> Result<Self, http_types::Error> {
        Ok(HTTP {
            client: surf::Client::with_client(Default::default()),
            app_name,
            instance_id,
            authorization,
            authorization_header: headers::HeaderName::from_bytes("authorization".into())?,
            app_name_header: headers::HeaderName::from_bytes("appname".into())?,
            instance_id_header: headers::HeaderName::from_bytes("instance_id".into())?,
        })
    }

    /// Perform a GET. Returns errors per surf::Client::get.
    pub fn get(&self, uri: impl AsRef<str>) -> surf::Request<C> {
        let request = self
            .client
            .get(uri)
            .set_header(self.app_name_header.clone(), self.app_name.as_str())
            .set_header(self.instance_id_header.clone(), self.instance_id.as_str());
        if let Some(auth) = &self.authorization {
            request.set_header(self.authorization_header.clone(), auth.as_str())
        } else {
            request
        }
    }

    /// Perform a GET. Returns errors per surf::Client::get.
    pub fn post(&self, uri: impl AsRef<str>) -> surf::Request<C> {
        let request = self
            .client
            .post(uri)
            .set_header(self.app_name_header.clone(), self.app_name.as_str())
            .set_header(self.instance_id_header.clone(), self.instance_id.as_str());
        if let Some(auth) = &self.authorization {
            request.set_header(self.authorization_header.clone(), auth.as_str())
        } else {
            request
        }
    }
}
