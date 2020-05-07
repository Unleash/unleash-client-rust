# Unleash API client for Rustlang

[Unleash](https://unleash.github.io) is a feature flag API system. This is a
client for it to facilitate using the API to control features in Rust programs.

## Client overview

The client is written using async. To use it in a sync program, run an async
executor and `block_on()` the relevant calls. As the client specification
requires sending background metrics to the API, you will need to arrange to
call the `poll` method from a thread. Contributions to provide helpers
to make this easier are welcome.

The unleash defined strategies are included, to support custom strategies
use the `ClientBuilder` and call the `strategy` method to register your custom
strategy memoization function.

## status

Core Unleash API features work.

Missing Unleash specified features:
- local serialised copy of toggles to survive restarts without network traffic.
- variant support.

Missing Rustlang features
- validation of the SDK with threaded code rather than pure async.

## Code of conduct

Please note that this project is released with a Contributor Code of Conduct. By
participating in this project you agree to abide by its terms.