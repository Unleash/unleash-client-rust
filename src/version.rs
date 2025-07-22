//! Managing SDK versions
//!
//! This module includes utilities to handle versioning aspects used internally
//! by the crate.
use std::env;

/// Returns the version of the `unleash-client-rust` SDK compiled into the binary.
///
/// The version number is included at compile time using the cargo package version
/// and is formatted as "unleash-client-rust:X.Y.Z", where X.Y.Z is the semantic
/// versioning format. This ensures a consistent versioning approach that aligns
/// with other Unleash SDKs.
pub(crate) fn get_sdk_version() -> &'static str {
    concat!("unleash-client-rust:", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[test]
    fn test_get_sdk_version_with_version_set() {
        let version_output = get_sdk_version();
        let version_regex = Regex::new(r"^unleash-client-rust:\d+\.\d+\.\d+$").unwrap();
        assert!(
            version_regex.is_match(version_output),
            "Version output did not match expected format: {version_output}"
        );
    }
}
