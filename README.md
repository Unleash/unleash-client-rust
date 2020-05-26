# Unleash API client for Rustlang

[Unleash](https://unleash.github.io) is a feature flag API system. This is a
client for it to facilitate using the API to control features in Rust programs.

## Client overview

The client is written using async. Any std compatible async runtime should be
compatible. Examples with async-std and tokio are in the examples/ in the source
tree.

To use it in a sync program, run an async executor and `block_on()` the relevant
calls. As the client specification requires sending background metrics to the
API, you will need to arrange to call the `poll_for_updates` method from a
thread as demonstrated in `examples/theads.rs`

The unleash defined strategies are included, to support custom strategies
use the `ClientBuilder` and call the `strategy` method to register your custom
strategy memoization function.

The crate documentation should be consulted for more detail.

## status

Core Unleash API features work, with Rust 1.42 or above.

Missing Unleash specified features:
- local serialised copy of toggles to survive restarts without network traffic.
- variant support.

## Code of conduct

Please note that this project is released with a Contributor Code of Conduct. By
participating in this project you agree to abide by its terms.

## Contributing

PR's on Github as normal please. Cargo test to run the test suite, rustfmt code
before submitting. To run the functional test suite:
```
docker-compose up -d
UNLEASH_API_URL=http://127.0.0.1:4242/api UNLEASH_APP_NAME=fred UNLEASH_INSTANCE_ID=test cargo test --features functional  -- --nocapture
```
or similar.
