use super::{BTreeMap, ConfigLoader, TlsPolicy};
use rstest::rstest;

#[rstest]
#[case::cipher_environment("", "SSL_SECURITY_LEVEL", "DEFAULT@SECLEVEL=2", "SSL_SECURITY_LEVEL")]
#[case::cipher_setting(
    "litellm_settings:\n  ssl_security_level: DEFAULT\n",
    "",
    "",
    "SSL_SECURITY_LEVEL"
)]
#[case::http2_wrong_type("", "LITELLM_HTTP2", "invalid", "LITELLM_HTTP2")]
fn http_policy_tests_behavior(
    #[case] yaml: &str,
    #[case] key: &str,
    #[case] value: &str,
    #[case] named: &str,
) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let environment = BTreeMap::from([(key.to_owned(), value.to_owned())]);
    assert!(
        TlsPolicy::resolve(&config, &environment, None)
            .unwrap_err()
            .to_string()
            .contains(named)
    );
}
