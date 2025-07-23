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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Context {
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub remote_address: Option<IPAddress>,
    #[serde(default, deserialize_with = "deserialize_context_properties")]
    pub properties: HashMap<String, String>,
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub environment: String,
    pub current_time: Option<DateTime<Utc>>,
}

fn deserialize_context_properties<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map = HashMap::<String, Option<String>>::deserialize(deserializer)?;
    let map: HashMap<String, String> = map
        .into_iter()
        .filter_map(|(key, value)| Some((key, value?)))
        .collect();
    Ok(map)
}
