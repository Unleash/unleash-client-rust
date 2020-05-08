// Copyright 2020 Cognite AS

// Run one thread per core, no updates. Given updates are once every 15 seconds,
// update frequency is effectively zero from an amortisation perspective.
// We could do a bench where we track the number of calls made and then
// introduce updates, to measure the impact of contention, but that will have
// higher in-loop costs as it has to record iterations, so this bench will still
// be useful.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use maplit::hashmap;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use unleash_api_client::api::{Feature, Features, Strategy};
use unleash_api_client::client;
use unleash_api_client::context::Context;

fn client(count: usize) -> client::Client<http_client::native::NativeClient> {
    let client = client::ClientBuilder::default()
        .into_client::<http_client::native::NativeClient>("notused", "app", "test", None)
        .unwrap();
    let mut features = vec![];
    for i in 0..count {
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
    client
}

fn batch(c: &mut Criterion) {
    let _ = simple_logger::init_with_level(log::Level::Warn);
    let cpus = num_cpus::get();
    let client = Arc::new(client(cpus));
    let iterations = 50_000;
    println!(
        "Benchmarking across {} threads with {} iterations per thread",
        cpus, iterations
    );
    let mut group = c.benchmark_group("batch");
    group
        .throughput(Throughput::Elements(iterations as u64))
        .warm_up_time(Duration::from_secs(15))
        .measurement_time(Duration::from_secs(30));
    group.bench_function("single thread", |b| {
        b.iter(|| {
            // Context creation is in here to make this comparable to parallel_same above.
            let context = Context {
                user_id: Some(thread_rng().sample_iter(&Alphanumeric).take(30).collect()),
                ..Default::default()
            };
            for _ in 0..iterations {
                client.is_enabled("flexible0", Some(&context), false);
            }
        })
    });
    group
        .throughput(Throughput::Elements(iterations * cpus as u64))
        .sample_size(10);
    group.bench_function("parallel same-feature", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for _cpu in 0..cpus {
                let thread_client = client.clone();
                let feature = format!("flexible0");
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
        })
    });
    group.bench_function("parallel distinct-features", |b| {
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
        })
    });
    group.bench_function("parallel unknown-features", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for cpu in 0..cpus {
                let thread_client = client.clone();
                let feature = format!("unknown{}", cpu);
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
        })
    });
    group.finish();
}

fn single_call(c: &mut Criterion) {
    let _ = simple_logger::init_with_level(log::Level::Warn);
    let client = client(1);
    let context = Context {
        user_id: Some(thread_rng().sample_iter(&Alphanumeric).take(30).collect()),
        ..Default::default()
    };
    let mut group = c.benchmark_group("single_call");
    group.throughput(Throughput::Elements(1));
    group.bench_function("single_call", |b| {
        b.iter(|| {
            client.is_enabled("flexible0", Some(&context), false);
        })
    });
    group.finish();
}

criterion_group!(benches, single_call, batch);
criterion_main!(benches);
