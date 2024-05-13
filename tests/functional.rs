// Copyright 2020 Cognite AS

//! Functional test against an unleashed API server running locally.
//! Set environment variables as per config.rs to exercise this.
//!
//! Currently expects a feature called default with one strategy default
//! Additional features are ignored.

#[cfg(feature = "functional")]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use std::{future::Future, pin::Pin};

    use async_std::task;
    use async_trait::async_trait;
    use enum_map::Enum;
    use futures_timer::Delay;
    use serde::{Deserialize, Serialize};

    use unleash_api_client::{client, config::EnvironmentConfig, http::HttpClient};

    #[cfg(not(any(feature = "surf", feature = "reqwest")))]
    compile_error!("Cannot run test suite without a client enabled");

    #[allow(non_camel_case_types)]
    #[derive(Debug, Deserialize, Serialize, Enum, Clone)]
    enum UserFeatures {
        default,
    }

    #[async_trait]
    trait AsyncImpl {
        type JoinHandle: Future<Output = ()>;
        fn spawn<F>(f: F) -> Self::JoinHandle
        where
            F: Future<Output = ()> + Send + 'static;

        async fn sleep(d: Duration);
    }

    #[cfg(feature = "surf")]
    struct AsyncStdAsync;
    #[cfg(feature = "surf")]
    #[async_trait]
    impl AsyncImpl for AsyncStdAsync {
        type JoinHandle = task::JoinHandle<()>;
        fn spawn<F>(f: F) -> Self::JoinHandle
        where
            F: Future<Output = ()> + Send + 'static,
        {
            task::spawn(f)
        }

        async fn sleep(d: Duration) {
            thread::sleep(d)
        }
    }

    #[cfg(or(feature = "reqwest", feature = "reqwest-11"))]
    struct TokioJoinHandle {
        inner: tokio::task::JoinHandle<()>,
    }

    impl Unpin for TokioJoinHandle {}

    impl Future for TokioJoinHandle {
        type Output = ();

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut task::Context<'_>,
        ) -> core::task::Poll<Self::Output> {
            let inner = Pin::new(&mut self.inner);
            match inner.poll(cx) {
                core::task::Poll::Pending => core::task::Poll::Pending,
                core::task::Poll::Ready(r) => core::task::Poll::Ready(r.unwrap()),
            }
        }
    }

    #[cfg(or(feature = "reqwest", feature = "reqwest-11"))]
    struct TokioAsync;
    #[cfg(or(feature = "reqwest", feature = "reqwest-11"))]
    #[async_trait]
    impl AsyncImpl for TokioAsync {
        type JoinHandle = TokioJoinHandle;
        fn spawn<F>(f: F) -> Self::JoinHandle
        where
            F: Future<Output = ()> + Send + 'static,
        {
            TokioJoinHandle {
                inner: tokio::spawn(f),
            }
        }

        async fn sleep(d: Duration) {
            tokio::time::sleep(d).await
        }
    }

    async fn test_smoke_async<C>() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        C: HttpClient + Default + 'static,
    {
        let _ = simple_logger::init();

        let config = EnvironmentConfig::from_env()?;
        let client = client::ClientBuilder::default()
            .interval(500)
            .into_client::<UserFeatures, C>(
                &config.api_url,
                &config.app_name,
                &config.instance_id,
                config.secret,
            )?;
        client.register().await?;
        futures::future::join(client.poll_for_updates(), async {
            // Ensure we have features
            Delay::new(Duration::from_millis(500)).await;
            assert!(client.is_enabled(UserFeatures::default, None, false));
            // Ensure the metrics get up-loaded
            Delay::new(Duration::from_millis(500)).await;
            client.stop_poll().await;
        })
        .await;
        println!("got metrics");
        Ok(())
    }

    #[cfg(feature = "surf")]
    #[test]
    fn test_smoke_async_surf() {
        task::block_on(async {
            test_smoke_async::<surf::Client>().await.unwrap();
        });
    }

    #[cfg(feature = "reqwest")]
    #[tokio::test]
    async fn test_smoke_async_reqwest() {
        test_smoke_async::<reqwest::Client>().await.unwrap();
    }
    #[cfg(feature = "reqwest-11")]
    #[tokio::test]
    async fn test_smoke_async_reqwest() {
        test_smoke_async::<reqwest_11::Client>().await.unwrap();
    }

    async fn test_smoke_threaded<C, A>(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        C: HttpClient + Default + 'static,
        A: AsyncImpl,
        <C as unleash_api_client::http::HttpClient>::RequestBuilder: std::marker::Send,
    {
        let _ = simple_logger::init();
        let config = EnvironmentConfig::from_env()?;
        let client = Arc::new(
            client::ClientBuilder::default()
                .interval(500)
                .into_client::<_, C>(
                    &config.api_url,
                    &config.app_name,
                    &config.instance_id,
                    config.secret,
                )?,
        );

        if let Err(e) = client.register().await {
            Err(e)
        } else {
            Ok(())
        }?;
        // Spin off a polling thread
        let poll_handle = client.clone();
        let handler = A::spawn(async move {
            // thread code
            poll_handle.poll_for_updates().await;
        });

        // Ensure we have features
        A::sleep(Duration::from_millis(500)).await;
        assert!(client.is_enabled(UserFeatures::default, None, false));
        // Ensure the metrics get up-loaded
        A::sleep(Duration::from_millis(500));
        client.stop_poll().await;

        handler.await;
        println!("got metrics");
        Ok(())
    }

    #[cfg(feature = "surf")]
    #[test]
    fn test_smoke_threaded_surf() {
        task::block_on(async {
            test_smoke_threaded::<surf::Client, AsyncStdAsync>()
                .await
                .unwrap();
        });
    }

    #[cfg(feature = "reqwest")]
    #[tokio::test]
    async fn test_smoke_threaded_reqwest() {
        test_smoke_threaded::<reqwest::Client, TokioAsync>()
            .await
            .unwrap();
    }
    #[cfg(feature = "reqwest-11")]
    #[tokio::test]
    async fn test_smoke_threaded_reqwest() {
        test_smoke_threaded::<reqwest_11::Client, TokioAsync>()
            .await
            .unwrap();
    }
}
