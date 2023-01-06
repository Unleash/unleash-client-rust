# Unleash API client for Rustlang

[Unleash](https://unleash.github.io) is a feature flag API system. This is a
client for it to facilitate using the API to control features in Rust programs.

## Client overview

The client is written using async rust. For communicating with the Unleash API
surf or reqwest support is built in, or any async HTTP client can be provided by
the user if they implement the thin trait used to abstract over the actual
client.

Examples with async-std and tokio are in the examples/ in the source
tree.

To use it in a sync program, run an async executor and `block_on()` the relevant
calls. As the client specification requires sending background metrics to the
API, you will need to arrange to call the `poll_for_updates` method from a
thread as demonstrated in `examples/theads.rs`

The unleash defined strategies are included, to support custom strategies
use the `ClientBuilder` and call the `strategy` method to register your custom
strategy memoization function.

The [crate documentation](https://docs.rs/unleash-api-client/latest/unleash_api_client/) should be consulted for more detail.

### Configuration

The easiest way to get started with the `Client` is using the `ClientBuilder`. A simple example is provided:

```rust
let config = EnvironmentConfig::from_env()?;
let client = client::ClientBuilder::default()
    .interval(500)
    .into_client::<UserFeatures, reqwest::Client>(
        &config.api_url,
        &config.app_name,
        &config.instance_id,
        config.secret,
    )?;
client.register().await?;
```

The values required for the `into_client` method are described as follows (in order, as seen above):

* `api_url` - The server URL to fetch toggles from.
* `app_name` - The name of your application.
* `instance_id` - A unique ID, ideally per run. A runtime generated UUID would be a sensible choice here.
* `authorization` - An Unleash client secret, if set this will be passed as the authorization header.

While the above code shows the usage of the `EnvironmentConfig`, this isn't required and is provided as a convenient way of reading a data from the system environment variables.

EnvironmentConfig Property | Environment Variable | Required? |
---------|-------------|-----------|
`api_url`  | `UNLEASH_API_URL`      | Yes |
`app_name` | `UNLEASH_APP_NAME`     | Yes |
`instance_id` | `UNLEASH_INSTANCE_ID` | Yes |
`secret` | `UNLEASH_CLIENT_SECRET` | No |

Note that if you do use the `EnvironmentConfig` as a way of accessing the system variables, you'll need to ensure that all the environment variables marked as required in the above table are set, or a panic will be raised.

The ClientBuilder also has a few builder methods for setting properties which are assumed to have good defaults and generally do not require changing. If you do need to alter these properties you can invoke the following methods on the builder (as seen above with the interval).

Method | Argument | Description | Default |
---------|-------------|-----------|-------|
interval  | u64 | Sets the polling interval to the Unleash server, in milliseconds | 15000ms |
disable_metric_submission | N/A | Turns off the metrics submission to Unleash | On |
enable_string_features | N/A | By default the Rust SDK requires you to define an enum for feature resolution, turning this on will allow you to resolve your features by string types instead, through the use of the `is_enabled_str` method. Be warned that this is enforced by asserts and calling `is_enabled_str` without turning this on with result in a panic | Off

## Status

Core Unleash API features work, with Rust 1.59 or above. The MSRV for this project is weakly enforced: when a hard dependency raises its version, so will the minimum version tested against, but if older rust versions work for a user, that is not prevented. `time` in particular is known to enforce a 6-month compiler age, so regular increases with the minimum version tested against are expected.

Unimplemented Unleash specified features:

* local serialised copy of toggles to survive restarts without network traffic.

## Code of conduct

Please note that this project is released with a Contributor Code of Conduct. By
participating in this project you agree to abide by its terms.

## Contributing

PR's on Github as normal please. Cargo test to run the test suite, rustfmt code
before submitting. To run the functional test suite you need an Unleash API to
execute against.

For instance, one way:

```shell
docker-compose up -d
```

Visit <http://localhost:4242/> and log in with admin + unleash4all, then create
a new API token at <http://localhost:4242/admin/api/create-token> for user
admin, type Client.

Then run the test suite:

```shell
UNLEASH_API_URL=http://127.0.0.1:4242/api \
  UNLEASH_APP_NAME=fred UNLEASH_INSTANCE_ID=test \
  UNLEASH_CLIENT_SECRET="<tokenvalue>" \
  cargo test --features functional  -- --nocapture
```

or similar. The functional test suite looks for a manually setup set of
features. E.g. log into the Unleash UI on port 4242 and create a feature called
`default`.
