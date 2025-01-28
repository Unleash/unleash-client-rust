use std::env;

// include version into the binary at compile time
pub fn get_sdk_version() -> &'static str {
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
            version_regex.is_match(&version_output),
            "Version output did not match expected format: {}",
            version_output
        );
    }
}
