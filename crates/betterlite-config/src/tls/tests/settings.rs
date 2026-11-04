use super::{BTreeMap, ConfigLoader, TlsPolicy, Verification};
use rstest::rstest;

#[rstest]
#[case::config_path("ssl_verify: ca.pem", &[("SSL_CERT_FILE", "other.pem")], Verification::CaFile("ca.pem".to_owned()))]
#[case::environment_path("ssl_verify: false", &[("SSL_VERIFY", "ca.pem")], Verification::CaFile("ca.pem".to_owned()))]
#[case::ca_override("ssl_verify: true", &[("SSL_CERT_FILE", "ca.pem")], Verification::CaFile("ca.pem".to_owned()))]
#[case::disabled_ignores_ca("ssl_verify: false", &[("SSL_CERT_FILE", "missing")], Verification::Disabled)]
#[case::empty_verify_environment("ssl_verify: false", &[("SSL_VERIFY", "")], Verification::Disabled)]
#[case::config_boolean_string("ssl_verify: 'false'", &[], Verification::Disabled)]
#[case::config_disabled("ssl_verify: false", &[], Verification::Disabled)]
fn http_policy_tests_behavior(
    #[case] settings: &str,
    #[case] env: &[(&str, &str)],
    #[case] expected: Verification,
) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!("litellm_settings:\n  {settings}\n"))
    .unwrap();
    let environment = env
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    assert_eq!(policy.verification, expected);
    assert_eq!(
        policy.ca_file,
        match expected {
            Verification::CaFile(path) => Some(path),
            Verification::Disabled | Verification::Enabled => None,
        }
    );
}
