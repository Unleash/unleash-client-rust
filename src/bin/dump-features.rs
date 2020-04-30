use std::env;

use async_std::task;

use unleash_api_client::http;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    task::block_on(async {
        let api_url = env::var("UNLEASH_API_URL");
        let endpoint = if let Ok(api_url) = api_url {
            format!("{}/client/features", api_url)
        } else {
            return Err(anyhow::anyhow!("UNLEASH_API_URL not set").into());
        };
        let secret = env::var("UNLEASH_CLIENT_SECRET").ok();
        let instance_id = if let Ok(instance_id) = env::var("UNLEASH_INSTANCE_ID") {
            instance_id
        } else {
            return Err(anyhow::anyhow!("UNLEASH_INSTANCE_ID not set").into());
        };
        let app_name = if let Ok(app_name) = env::var("UNLEASH_APP_NAME") {
            app_name
        } else {
            return Err(anyhow::anyhow!("UNLEASH_APP_NAME not set").into());
        };

        let client: http::HTTP<http_client::native::NativeClient> =
            http::HTTP::new(app_name, instance_id, secret)?;
        let mut res = client.get(endpoint).await?;
        dbg!(res.body_string().await?);
        Ok(())
    })
}
