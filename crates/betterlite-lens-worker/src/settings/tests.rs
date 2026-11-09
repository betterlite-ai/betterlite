#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{SettingsError, WorkerSettings};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::proxy_url("LITELLM_URL")]
#[case::worker_token("LENS_WORKER_TOKEN")]
fn refuses_to_start_without_a_required_variable(#[case] missing: &str) {
    let mut variables = environment(&[
        ("LITELLM_URL", "http://proxy"),
        ("LENS_WORKER_TOKEN", "worker-token"),
    ]);
    variables.remove(missing);
    let error = WorkerSettings::resolve(&variables).unwrap_err();
    assert!(matches!(error, SettingsError::Missing { name } if name == missing));
    assert!(error.to_string().contains(missing));
    assert!(!error.to_string().contains("worker-token"));
}

#[rstest]
#[case::proxy_url("LITELLM_URL")]
#[case::worker_token("LENS_WORKER_TOKEN")]
fn treats_an_empty_value_as_unset(#[case] empty: &str) {
    let mut variables = environment(&[
        ("LITELLM_URL", "http://proxy"),
        ("LENS_WORKER_TOKEN", "worker-token"),
    ]);
    variables.insert(empty.to_owned(), String::new());
    let error = WorkerSettings::resolve(&variables).unwrap_err();
    assert!(matches!(error, SettingsError::Missing { name } if name == empty));
}

#[test]
fn removes_every_trailing_slash_from_the_url() {
    let settings = WorkerSettings::resolve(&environment(&[
        ("LITELLM_URL", "http://proxy.example:4000///"),
        ("LENS_WORKER_TOKEN", "worker-token"),
    ]))
    .unwrap();
    assert_eq!(settings.litellm_url, "http://proxy.example:4000");
}

#[test]
fn keeps_the_token_out_of_debug_output() {
    let settings = WorkerSettings::resolve(&environment(&[
        ("LITELLM_URL", "http://proxy"),
        ("LENS_WORKER_TOKEN", "worker-token"),
    ]))
    .unwrap();
    assert_eq!(settings.token.expose_secret(), "worker-token");
    let debug = format!("{:?}", settings.token);
    assert!(!debug.contains("worker-token"));
    assert!(debug.contains("REDACTED"));
}
