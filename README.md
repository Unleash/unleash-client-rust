# Unleash API client for Rustlang

[Unleash](https://unleash.github.io) is a feature flag API system. This is a
client for it to facilitate using the API to control features in Rust programs.

## Client overview

The client is written using async. To use it in a sync program, run an async
executor and `block_on()` the relevant calls. As the client specification
requires sending background metrics to the API, you will need to arrange to
call the `submit_metrics` method periodically.

The unleash defined strategies are included, to support custom strategies implement
the Strategy trait and insert the strategy into the Unleash.strategies collection.

## status

Current status - in development. 
