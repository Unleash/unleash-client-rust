// Copyright 2020 Cognite AS
use async_std::task;

use unleash_api_client::api;
use unleash_api_client::config::EnvironmentConfig;
use unleash_api_client::http;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    task::block_on(async {
        let config = EnvironmentConfig::from_env()?;
        let endpoint = api::Features::endpoint(&config.api_url, None);
        let client: http::HTTP<surf::Client> =
            http::HTTP::new(config.app_name, config.instance_id, config.secret)?;
        let res: api::Features = client.get_json(&endpoint).await?;
        dbg!(res);
        Ok(())
    })
}
