// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/user_guide/unleash_context>
use chrono::Utc;
use std::{collections::HashMap, net::IpAddr};

use chrono::DateTime;
use serde::{de, Deserialize};

// Custom IP Address newtype that can be deserialised from strings e.g. 127.0.0.1 for use with tests.
#[derive(Debug)]
pub struct IPAddress(pub IpAddr);

impl<'de> de::Deserialize<'de> for IPAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            // Deserialize from a human-readable string like "127.0.0.1".
            let s = String::deserialize(deserializer)?;
            s.parse::<IpAddr>()
                .map_err(de::Error::custom)
                .map(IPAddress)
        } else {
            unimplemented!();
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "remoteAddress")]
    pub remote_address: Option<IPAddress>,
    #[serde(default)]
    pub properties: HashMap<String, String>,
    #[serde(default, rename = "appName")]
    pub app_name: String,
    #[serde(default)]
    pub environment: String,
    /// Defaults to the current time.
    #[serde(rename = "currentTime", default = "Utc::now")]
    pub current_time: DateTime<Utc>,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            user_id: Default::default(),
            session_id: Default::default(),
            remote_address: Default::default(),
            properties: Default::default(),
            app_name: Default::default(),
            environment: Default::default(),
            current_time: Utc::now(),
        }
    }
}
