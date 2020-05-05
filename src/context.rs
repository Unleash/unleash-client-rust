// Copyright 2020 Cognite AS
//! https://unleash.github.io/docs/unleash_context
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Context {
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub remote_address: Option<ipaddress::IPAddress>,
    pub properties: HashMap<String, String>,
    pub app_name: String,
    pub environment: String,
}
