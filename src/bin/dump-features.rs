// Copyright 2020 Cognite AS
use async_std::task;

use unleash_api_client::api;
use unleash_api_client::config::EnvironmentConfig;
use unleash_api_client::http;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    task::block_on(async {
        let config = EnvironmentConfig::from_env()?;
        let endpoint = api::Features::endpoint(&config.api_url);
        let client: http::HTTP =
            http::HTTP::new(config.app_name, config.instance_id, config.secret)?;
        let res: api::Features = client.get(endpoint).recv_json().await?;
        dbg!(res);
        Ok(())
    })
}
