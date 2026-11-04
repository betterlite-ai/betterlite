#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::tls::{TlsPolicy, Verification, resolve_ca_file};

#[rstest]
#[case::defaults("", BTreeMap::new(), None, Verification::Enabled)]
#[case::environment_disables("", BTreeMap::from([("SSL_VERIFY".to_owned(), "false".to_owned())]), None, Verification::Disabled)]
#[case::environment_over_config("litellm_settings:\n  ssl_verify: true\n", BTreeMap::from([("SSL_VERIFY".to_owned(), "false".to_owned())]), None, Verification::Disabled)]
#[case::argument_wins("litellm_settings:\n  ssl_verify: false\n", BTreeMap::from([("SSL_VERIFY".to_owned(), "true".to_owned())]), Some(true), Verification::Enabled)]
#[case::deployment_from_yaml(
    "model_list:\n  - model_name: test\n    litellm_params:\n      model: provider/model\n      ssl_verify: false\n",
    BTreeMap::new(),
    None,
    Verification::Disabled
)]
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
    let deployment_ssl_verify = config
        .model_list
        .first()
        .and_then(|model_deployment| model_deployment.litellm_params.ssl_verify)
        .or(deployment);
    let policy = TlsPolicy::resolve(&config, &environment, deployment_ssl_verify).unwrap();
    assert_eq!(policy.verification, expected);
    let http2_config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  http2: true\n")
    .unwrap();
    let disabled_http2_environment =
        BTreeMap::from([("LITELLM_HTTP2".to_owned(), "false".to_owned())]);
    let config_http2_policy =
        TlsPolicy::resolve(&http2_config, &disabled_http2_environment, None).unwrap();
    assert!(config_http2_policy.http2);
    let enabled_http2_environment =
        BTreeMap::from([("LITELLM_HTTP2".to_owned(), "true".to_owned())]);
    let environment_http2_policy =
        TlsPolicy::resolve(&config, &enabled_http2_environment, None).unwrap();
    assert!(environment_http2_policy.http2);
    assert_eq!(
        resolve_ca_file(Verification::Enabled, Some(&"/dev/null".to_owned())),
        Verification::CaFile("/dev/null".to_owned())
    );
    assert_eq!(
        resolve_ca_file(Verification::Disabled, Some(&"/dev/null".to_owned())),
        Verification::Disabled
    );
}
