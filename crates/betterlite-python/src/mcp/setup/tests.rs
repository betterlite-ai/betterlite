#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::tls::{Verification, VerifySetting};

use super::{proxy_policy, tls_policy};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn setup_explicit_ssl_verify_beats_the_environment() {
    let policy = tls_policy(
        Some(&VerifySetting::Bool(true)),
        &environment(&[("SSL_VERIFY", "false")]),
    )
    .unwrap();
    assert_eq!(policy.verification, Verification::Enabled);
}

#[test]
fn setup_environment_ssl_verify_is_used_without_an_explicit_one() {
    let policy = tls_policy(None, &environment(&[("SSL_VERIFY", "false")])).unwrap();
    assert_eq!(policy.verification, Verification::Disabled);
}

#[test]
fn setup_explicit_false_disables_verification() {
    let policy = tls_policy(Some(&VerifySetting::Bool(false)), &BTreeMap::new()).unwrap();
    assert_eq!(policy.verification, Verification::Disabled);
}

#[test]
fn setup_ca_file_path_verifies_against_it() {
    let policy = tls_policy(
        Some(&VerifySetting::Path("/etc/ca.pem".to_owned())),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        policy.verification,
        Verification::CaFile("/etc/ca.pem".to_owned())
    );
}

#[test]
fn setup_ssl_certificate_is_carried() {
    let policy = tls_policy(
        None,
        &environment(&[("SSL_CERTIFICATE", "/etc/client.pem")]),
    )
    .unwrap();
    assert_eq!(policy.certificate.as_deref(), Some("/etc/client.pem"));
}

#[test]
fn setup_ssl_security_level_is_refused() {
    let failure = tls_policy(None, &environment(&[("SSL_SECURITY_LEVEL", "2")])).unwrap_err();
    assert!(failure.message.contains("SSL_SECURITY_LEVEL"));
}

#[test]
fn setup_proxy_policy_reads_the_environment() {
    let policy = proxy_policy(&environment(&[("HTTPS_PROXY", "http://proxy.test:8080")])).unwrap();
    assert!(policy.primary.contains_key("https"));
}

#[test]
fn setup_proxy_policy_is_empty_without_variables() {
    let policy = proxy_policy(&BTreeMap::new()).unwrap();
    assert!(policy.primary.is_empty());
}
