use std::collections::BTreeMap;
use std::sync::Arc;

use rstest::rstest;
use rustls::NamedGroup;
use rustls::client::danger::ServerCertVerifier as _;
use rustls::crypto::aws_lc_rs;

use super::*;
use crate::tls::{InsecureCertificateVerifier, TlsError, named_group, select_kx_groups};

#[rstest]
#[case::x25519("X25519")]
#[case::prime256v1("prime256v1")]
#[case::secp384r1("secp384r1")]
#[case::invalid_falls_back("invalid")]
#[case::unsupported_provider_group("unsupported:X25519")]
fn http_policy_tests_behavior(#[case] curve: &str) {
    if let Some(name) = curve.strip_prefix("unsupported:") {
        let mut provider = aws_lc_rs::default_provider();
        provider.kx_groups.clear();
        let result = select_kx_groups(NamedGroup::X25519, name, &provider);
        assert!(matches!(result, Err(TlsError::UnsupportedCurve(found)) if found == name));
        return;
    }
    let expected_group = match curve {
        "X25519" => Some(NamedGroup::X25519),
        "prime256v1" => Some(NamedGroup::secp256r1),
        "secp384r1" => Some(NamedGroup::secp384r1),
        _ => None,
    };
    assert_eq!(named_group(curve), expected_group);
    if let Some(group) = expected_group {
        let provider = aws_lc_rs::default_provider();
        let groups = select_kx_groups(group, curve, &provider).unwrap();
        assert_eq!(groups.first().map(|selected| selected.name()), Some(group));
    }
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    let environment = BTreeMap::from([("SSL_ECDH_CURVE".to_owned(), curve.to_owned())]);
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let builder = apply_policy(ClientBuilder::new(), &policy).unwrap();
    let _client = builder.build().unwrap();
}

#[test]
fn insecure_verifier_supports_provider_signature_schemes() {
    let algorithms = aws_lc_rs::default_provider().signature_verification_algorithms;
    let verifier = InsecureCertificateVerifier {
        algorithms: Arc::new(algorithms),
    };
    assert!(!verifier.supported_verify_schemes().is_empty());
}
