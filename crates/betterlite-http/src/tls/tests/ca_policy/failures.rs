use super::{BTreeMap, ClientBuilder, ConfigLoader, TlsPolicy, apply_policy};
use rstest::rstest;

#[rstest]
#[case::unreadable_ca("SSL_CERT_FILE", "missing-ca.pem", "SSL_CERT_FILE")]
#[case::invalid_header("LITELLM_USER_AGENT", "private\r\nheader", "LITELLM_USER_AGENT")]
#[case::unsupported_curve("SSL_ECDH_CURVE", "secp521r1", "SSL_ECDH_CURVE secp521r1")]
fn http_policy_tests_behavior(#[case] key: &str, #[case] value: &str, #[case] named: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    let environment = BTreeMap::from([(key.to_owned(), value.to_owned())]);
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let error = apply_policy(ClientBuilder::new(), &policy).unwrap_err();
    assert!(error.to_string().contains(named), "{error}");
    assert!(!format!("{error:?}").contains("private"));
}
