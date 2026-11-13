use super::{BTreeMap, ConfigLoader, TlsPolicy};
use rstest::rstest;

#[rstest]
#[case::curve_environment_wins("ssl_ecdh_curve", "SSL_ECDH_CURVE", "environment", "environment")]
#[case::certificate_environment_wins(
    "ssl_certificate",
    "SSL_CERTIFICATE",
    "environment",
    "environment"
)]
#[case::config_curve_fallback("ssl_ecdh_curve", "SSL_ECDH_CURVE", "", "configured")]
#[case::config_certificate_fallback("ssl_certificate", "SSL_CERTIFICATE", "", "configured")]
fn http_policy_tests_behavior(
    #[case] key: &str,
    #[case] variable: &str,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!("litellm_settings:\n  {key}: configured\n"))
    .unwrap();
    let environment = BTreeMap::from([(variable.to_owned(), value.to_owned())]);
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let actual = if key == "ssl_ecdh_curve" {
        policy.ecdh_curve
    } else {
        policy.certificate
    };
    assert_eq!(actual.as_deref(), Some(expected));
}
