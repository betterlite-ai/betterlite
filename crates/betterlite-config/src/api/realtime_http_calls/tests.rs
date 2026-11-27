#![cfg(test)]
use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

mod loads_realtime_http_settings;

const DEFAULTS: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-http-calls-default.yaml");
const CONFIGURED: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-http-calls-configured.yaml");
// B198's constants module reads REQUEST_TIMEOUT once, 6000 seconds unless the environment says otherwise
const DEFAULT_REQUEST_TIMEOUT_SECONDS: f64 = 6000.0;

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}
