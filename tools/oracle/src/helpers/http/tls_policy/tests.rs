#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Input, Op};
use crate::helpers::Helper as _;

#[rstest]
#[case::invalid_curve("invalid", None, true)]
#[case::valid_curve("X25519", None, false)]
#[case::disabled_invalid_curve("invalid", Some(false), false)]
fn http_policy_tests_behavior(
    #[case] curve: &str,
    #[case] verification: Option<bool>,
    #[case] fallback: bool,
) {
    let input = Input {
        ca_pem: None,
        deployment_ssl_verify: verification,
        config_ssl_verify: None,
        config_http2: None,
        config_ssl_certificate: None,
        environment: BTreeMap::from([("SSL_ECDH_CURVE".to_owned(), curve.to_owned())]),
    };
    let result = Op::run(&input).unwrap();
    assert_eq!(
        result.get("curve_fallback"),
        Some(&serde_json::json!(fallback))
    );
}

#[rstest]
#[case::configured_certificate(None, "configured-client.pem")]
#[case::environment_over_configuration(Some("environment-client.pem"), "environment-client.pem")]
fn certificate_precedence(#[case] environment: Option<&str>, #[case] expected: &str) {
    let input = Input {
        ca_pem: None,
        deployment_ssl_verify: None,
        config_ssl_verify: None,
        config_http2: None,
        config_ssl_certificate: Some("configured-client.pem".to_owned()),
        environment: environment
            .map(|value| ("SSL_CERTIFICATE".to_owned(), value.to_owned()))
            .into_iter()
            .collect(),
    };
    let result = Op::run(&input).unwrap();
    assert_eq!(
        result.get("certificate"),
        Some(&serde_json::json!(expected))
    );
}
