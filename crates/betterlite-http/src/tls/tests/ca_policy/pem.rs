use crate::tls::parse_ca_pem;
use rstest::rstest;

#[rstest]
#[case::empty(b"")]
#[case::malformed(b"-----BEGIN CERTIFICATE-----\nsecret-not-base64\n-----END CERTIFICATE-----")]
#[case::invalid_der(b"-----BEGIN CERTIFICATE-----\neA==\n-----END CERTIFICATE-----")]
fn http_policy_tests_behavior(#[case] bytes: &[u8]) {
    let error = parse_ca_pem(bytes).unwrap_err();
    assert!(error.to_string().contains("SSL_CERT_FILE"));
    assert!(!format!("{error:?}").contains("secret-not-base64"));
}
