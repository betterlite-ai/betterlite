#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::tls::{TlsPolicy, Verification};

#[rstest]
#[case::defaults("", BTreeMap::new(), None, Verification::Enabled)]
#[case::environment_disables("", BTreeMap::from([("SSL_VERIFY".to_owned(), "false".to_owned())]), None, Verification::Disabled)]
#[case::argument_wins("litellm_settings:\n  ssl_verify: false\n", BTreeMap::from([("SSL_VERIFY".to_owned(), "true".to_owned())]), Some(true), Verification::Enabled)]
fn http_policy_tests_behavior(
    #[case] yaml: &str,
    #[case] environment: BTreeMap<String, String>,
    #[case] deployment: Option<bool>,
    #[case] expected: Verification,
) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let policy = TlsPolicy::resolve(&config, &environment, deployment).unwrap();
    assert_eq!(policy.verification, expected);
}
