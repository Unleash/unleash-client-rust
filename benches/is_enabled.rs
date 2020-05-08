// Copyright 2020 Cognite AS

// Run one thread per core, no updates. Given updates are once every 15 seconds,
// update frequency is effectively zero from an amortisation perspective.
// We could do a bench where we track the number of calls made and then
// introduce updates, to measure the impact of contention, but that will have
// higher in-loop costs as it has to record iterations, so this bench will still
// be useful.

#![feature(test)]
extern crate test;

use std::sync::Arc;
use std::thread;
use test::Bencher;

use maplit::hashmap;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use unleash_api_client::api::{Feature, Features, Strategy};
use unleash_api_client::client;
use unleash_api_client::context::Context;

fn client(
    count: usize,
) -> Result<
    client::Client<http_client::native::NativeClient>,
    Box<dyn std::error::Error + Send + Sync + 'static>,
> {
    let client = client::ClientBuilder::default()
        .into_client::<http_client::native::NativeClient>("notused", "app", "test", None)?;
    let mut features = vec![];
    for i in 1..count + 1 {
        let name = format!("flexible{}", i);
        features.push(Feature {
            description: name.clone(),
            enabled: true,
            created_at: None,
            variants: None,
            name: name,
            strategies: vec![Strategy {
                name: "flexibleRollout".into(),
                parameters: Some(hashmap!["stickiness".into()=>"DEFAULT".into(),
                    "groupId".into()=>"flexible".into(), "rollout".into()=>"33".into()]),
            }],
        });
    }
    let f = Features {
        version: 1,
        features,
    };
    client.memoize(f.features).unwrap();
    Ok(client)
}

#[bench]
fn parallel_same(
    b: &mut Bencher,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let _ = simple_logger::init_with_level(log::Level::Warn);
    let cpus = num_cpus::get();
    let client = Arc::new(client(cpus)?);
    let iterations = 10_000;
    println!(
        "Benchmarking across {} threads with {} iterations per thread",
        cpus, iterations
    );
    b.iter(|| {
        let mut threads = vec![];
        for _cpu in 0..cpus {
            let thread_client = client.clone();
            let feature = format!("flexible1");
            let handle = thread::spawn(move || {
                let context = Context {
                    user_id: Some(thread_rng().sample_iter(&Alphanumeric).take(30).collect()),
                    ..Default::default()
                };
                for _ in 0..iterations {
                    thread_client.is_enabled(&feature, Some(&context), false);
                }
            });
            threads.push(handle);
        }
        for thread in threads {
            thread.join().unwrap();
        }
    });
    Ok(())
}

#[bench]
fn parallel_sharded(
    b: &mut Bencher,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let _ = simple_logger::init_with_level(log::Level::Warn);
    let cpus = num_cpus::get();
    let client = Arc::new(client(cpus)?);
    let iterations = 10_000;
    println!(
        "Benchmarking across {} threads with {} iterations per thread",
        cpus, iterations
    );
    b.iter(|| {
        let mut threads = vec![];
        for cpu in 0..cpus {
            let thread_client = client.clone();
            let feature = format!("flexible{}", cpu);
            let handle = thread::spawn(move || {
                let context = Context {
                    user_id: Some(thread_rng().sample_iter(&Alphanumeric).take(30).collect()),
                    ..Default::default()
                };
                for _ in 0..iterations {
                    thread_client.is_enabled(&feature, Some(&context), false);
                }
            });
            threads.push(handle);
        }
        for thread in threads {
            thread.join().unwrap();
        }
    });
    Ok(())
}

#[bench]
fn single_loop(b: &mut Bencher) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let _ = simple_logger::init_with_level(log::Level::Warn);
    let cpus = num_cpus::get();
    let client = client(cpus)?;
    let iterations = 10_000;
    println!(
        "Benchmarking across 1 threads with {} iterations per thread, {} defined features",
        iterations, cpus
    );
    b.iter(|| {
        // Context creation is in here to make this comparable to parallel_same above.
        let context = Context {
            user_id: Some(thread_rng().sample_iter(&Alphanumeric).take(30).collect()),
            ..Default::default()
        };
        for _ in 0..iterations {
            client.is_enabled("flexible1", Some(&context), false);
        }
    });
    Ok(())
}

#[bench]
fn single_call(b: &mut Bencher) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let _ = simple_logger::init_with_level(log::Level::Warn);
    let client = client(1)?;
    let context = Context {
        user_id: Some(thread_rng().sample_iter(&Alphanumeric).take(30).collect()),
        ..Default::default()
    };
    b.iter(|| {
        client.is_enabled("flexible1", Some(&context), false);
    });
    Ok(())
}
