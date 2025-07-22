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
use enum_map::Enum;
use maplit::hashmap;
use rand::{distr::Alphanumeric, rng, Rng};
use serde::{Deserialize, Serialize};

use unleash_api_client::api::{Feature, Features, Strategy};
use unleash_api_client::client;
use unleash_api_client::context::Context;
use unleash_api_client::http::HttpClient;

// TODO: do a build.rs thing to determine available CPU count at build time for
// optimal vec sizing.

#[allow(non_camel_case_types)]
#[derive(Debug, Deserialize, Serialize, Enum, Clone)]
enum UserFeatures {
    Flexible0,
    Flexible1,
    Flexible2,
    Flexible3,
    Flexible4,
    Flexible5,
    Flexible6,
    Flexible7,
    Flexible8,
    Flexible9,
    Flexible10,
    Flexible11,
    Flexible12,
    Flexible13,
    Flexible14,
    Flexible15,
    Flexible16,
    Flexible17,
    Flexible18,
    Flexible19,
    Flexible20,
    Flexible21,
    Flexible22,
    Flexible23,
    Flexible24,
    Flexible25,
    Flexible26,
    Flexible27,
    Flexible28,
    Flexible29,
    Flexible30,
    Flexible31,
    Flexible32,
    Flexible33,
    Flexible34,
    Flexible35,
    Flexible36,
    Flexible37,
    Flexible38,
    Flexible39,
    Flexible40,
    Flexible41,
    Flexible42,
    Flexible43,
    Flexible44,
    Flexible45,
    Flexible46,
    Flexible47,
    Flexible48,
    Flexible49,
    Flexible50,
    Flexible51,
    Flexible52,
    Flexible53,
    Flexible54,
    Flexible55,
    Flexible56,
    Flexible57,
    Flexible58,
    Flexible59,
    Flexible60,
    Flexible61,
    Flexible62,
    Flexible63,
    Unknown0,
    Unknown1,
    Unknown2,
    Unknown3,
    Unknown4,
    Unknown5,
    Unknown6,
    Unknown7,
    Unknown8,
    Unknown9,
    Unknown10,
    Unknown11,
    Unknown12,
    Unknown13,
    Unknown14,
    Unknown15,
    Unknown16,
    Unknown17,
    Unknown18,
    Unknown19,
    Unknown20,
    Unknown21,
    Unknown22,
    Unknown23,
    Unknown24,
    Unknown25,
    Unknown26,
    Unknown27,
    Unknown28,
    Unknown29,
    Unknown30,
    Unknown31,
    Unknown32,
    Unknown33,
    Unknown34,
    Unknown35,
    Unknown36,
    Unknown37,
    Unknown38,
    Unknown39,
    Unknown40,
    Unknown41,
    Unknown42,
    Unknown43,
    Unknown44,
    Unknown45,
    Unknown46,
    Unknown47,
    Unknown48,
    Unknown49,
    Unknown50,
    Unknown51,
    Unknown52,
    Unknown53,
    Unknown54,
    Unknown55,
    Unknown56,
    Unknown57,
    Unknown58,
    Unknown59,
    Unknown60,
    Unknown61,
    Unknown62,
    Unknown63,
}

fn client<C>(count: usize) -> client::Client<UserFeatures, C>
where
    C: HttpClient + Default,
{
    let client = client::ClientBuilder::default()
        .enable_string_features()
        .into_client::<UserFeatures, C>("notused", "app", "test", None)
        .unwrap();
    let mut features = vec![];
    for i in 0..count {
        // once for enums, once for strings
        let name = format!("Flexible{i}");
        features.push(Feature {
            description: Some(name.clone()),
            enabled: true,
            created_at: None,
            variants: None,
            name,
            strategies: vec![Strategy {
                name: "flexibleRollout".into(),
                parameters: Some(hashmap!["stickiness".into()=>"default".into(),
                    "groupId".into()=>"flexible".into(), "rollout".into()=>"33".into()]),
                ..Default::default()
            }],
        });
        let name = format!("flexible{i}");
        features.push(Feature {
            description: Some(name.clone()),
            enabled: true,
            created_at: None,
            variants: None,
            name,
            strategies: vec![Strategy {
                name: "flexibleRollout".into(),
                parameters: Some(hashmap!["stickiness".into()=>"default".into(),
                    "groupId".into()=>"flexible".into(), "rollout".into()=>"33".into()]),
                ..Default::default()
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

#[inline]
fn random_str() -> String {
    rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect()
}

fn batch(c: &mut Criterion) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "reqwest")] {
            use reqwest::Client as HttpClient;
        } else if #[cfg(feature = "reqwest-11")] {
            use reqwest_11::Client as HttpClient;
        } else {
            compile_error!("Cannot run test suite without a client enabled");
        }
    }
    let _ = simple_logger::SimpleLogger::new()
        .with_utc_timestamps()
        .with_module_level("isahc::agent", log::LevelFilter::Off)
        .with_module_level("tracing::span", log::LevelFilter::Off)
        .with_module_level("tracing::span::active", log::LevelFilter::Off)
        .with_level(log::LevelFilter::Warn)
        .init();
    let cpus = num_cpus::get();
    let client = Arc::new(client::<HttpClient>(cpus));
    let iterations = 50_000;
    println!("Benchmarking across {cpus} threads with {iterations} iterations per thread");
    let mut group = c.benchmark_group("batch");
    group
        .throughput(Throughput::Elements(iterations))
        .warm_up_time(Duration::from_secs(15))
        .measurement_time(Duration::from_secs(30));
    group.bench_function("single thread(enum)", |b| {
        b.iter(|| {
            // Context creation is in here to make this comparable to parallel_same above.
            let context = Context {
                user_id: Some(random_str()),
                ..Default::default()
            };
            for _ in 0..iterations {
                client.is_enabled(UserFeatures::Flexible0, Some(&context), false);
            }
        })
    });
    group.bench_function("single thread(str)", |b| {
        b.iter(|| {
            // Context creation is in here to make this comparable to parallel_same above.
            let context = Context {
                user_id: Some(random_str()),
                ..Default::default()
            };
            for _ in 0..iterations {
                client.is_enabled_str("flexible0", Some(&context), false);
            }
        })
    });
    group
        .throughput(Throughput::Elements(iterations * cpus as u64))
        .sample_size(10);
    group.bench_function("parallel same-feature(enum)", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for _cpu in 0..cpus {
                let thread_client = client.clone();
                let feature = UserFeatures::Flexible0;
                let handle = thread::spawn(move || {
                    let context = Context {
                        user_id: Some(random_str()),
                        ..Default::default()
                    };
                    for _ in 0..iterations {
                        thread_client.is_enabled(feature.clone(), Some(&context), false);
                    }
                });
                threads.push(handle);
            }
            for thread in threads {
                thread.join().unwrap();
            }
        })
    });
    group.bench_function("parallel same-feature(str)", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for _cpu in 0..cpus {
                let thread_client = client.clone();
                let feature = "flexible0";
                let handle = thread::spawn(move || {
                    let context = Context {
                        user_id: Some(random_str()),
                        ..Default::default()
                    };
                    for _ in 0..iterations {
                        thread_client.is_enabled_str(feature, Some(&context), false);
                    }
                });
                threads.push(handle);
            }
            for thread in threads {
                thread.join().unwrap();
            }
        })
    });

    group.bench_function("parallel distinct-features(enum)", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for cpu in 0..cpus {
                let thread_client = client.clone();
                let feature_str = format!("Flexible{cpu}");
                let feature = serde_plain::from_str::<UserFeatures>(&feature_str).unwrap();
                let handle = thread::spawn(move || {
                    let context = Context {
                        user_id: Some(random_str()),
                        ..Default::default()
                    };
                    for _ in 0..iterations {
                        thread_client.is_enabled(feature.clone(), Some(&context), false);
                    }
                });
                threads.push(handle);
            }
            for thread in threads {
                thread.join().unwrap();
            }
        })
    });
    group.bench_function("parallel distinct-features(str)", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for cpu in 0..cpus {
                let thread_client = client.clone();
                let feature_str = format!("flexible{cpu}");
                let handle = thread::spawn(move || {
                    let context = Context {
                        user_id: Some(random_str()),
                        ..Default::default()
                    };
                    for _ in 0..iterations {
                        thread_client.is_enabled_str(&feature_str, Some(&context), false);
                    }
                });
                threads.push(handle);
            }
            for thread in threads {
                thread.join().unwrap();
            }
        })
    });
    group.bench_function("parallel unknown-features(enum)", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for cpu in 0..cpus {
                let thread_client = client.clone();
                let feature_str = format!("Unknown{cpu}");
                let feature = serde_plain::from_str::<UserFeatures>(&feature_str).unwrap();
                let handle = thread::spawn(move || {
                    let context = Context {
                        user_id: Some(random_str()),
                        ..Default::default()
                    };
                    for _ in 0..iterations {
                        thread_client.is_enabled(feature.clone(), Some(&context), false);
                    }
                });
                threads.push(handle);
            }
            for thread in threads {
                thread.join().unwrap();
            }
        })
    });
    group.bench_function("parallel unknown-features(str)", |b| {
        b.iter(|| {
            let mut threads = vec![];
            for cpu in 0..cpus {
                let thread_client = client.clone();
                let feature_str = format!("unknown{cpu}");
                let handle = thread::spawn(move || {
                    let context = Context {
                        user_id: Some(random_str()),
                        ..Default::default()
                    };
                    for _ in 0..iterations {
                        thread_client.is_enabled_str(&feature_str, Some(&context), false);
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
    let _ = simple_logger::SimpleLogger::new()
        .with_utc_timestamps()
        .with_module_level("isahc::agent", log::LevelFilter::Off)
        .with_module_level("tracing::span", log::LevelFilter::Off)
        .with_module_level("tracing::span::active", log::LevelFilter::Off)
        .with_level(log::LevelFilter::Warn)
        .init();
    cfg_if::cfg_if! {
        if #[cfg(feature = "reqwest")] {
            use reqwest::Client as HttpClient;
        } else if #[cfg(feature = "reqwest-11")] {
            use reqwest_11::Client as HttpClient;
        } else {
            compile_error!("Cannot run test suite without a client enabled");
        }
    }
    let client = client::<HttpClient>(1);
    let context = Context {
        user_id: Some(random_str()),
        ..Default::default()
    };
    let mut group = c.benchmark_group("single_call");
    group.throughput(Throughput::Elements(1));
    group.bench_function("single_call(enum)", |b| {
        b.iter(|| {
            client.is_enabled(UserFeatures::Flexible0, Some(&context), false);
        })
    });
    group.bench_function("single_call(str)", |b| {
        b.iter(|| {
            client.is_enabled_str("flexible0", Some(&context), false);
        })
    });

    group.finish();
}

criterion_group!(benches, single_call, batch);
criterion_main!(benches);
