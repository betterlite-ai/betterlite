use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::TlsPolicy;
use rstest::rstest;
use rustls::crypto::aws_lc_rs;

use crate::tls::configure_curve;

#[rstest]
#[case::default_groups(None, None)]
#[case::disabled_invalid(Some(false), Some("invalid"))]
#[case::disabled_unsupported(Some(false), Some("X448"))]
fn http_policy_tests_behavior(#[case] verify: Option<bool>, #[case] curve: Option<&str>) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    let environment = curve
        .map(|value| ("SSL_ECDH_CURVE".to_owned(), value.to_owned()))
        .into_iter()
        .collect();
    let policy = TlsPolicy::resolve(&config, &environment, verify).unwrap();
    let (provider, fallback) = configure_curve(&policy).unwrap();
    assert!(!fallback);
    let offered: Vec<_> = provider
        .kx_groups
        .iter()
        .map(|group| group.name())
        .collect();
    let defaults: Vec<_> = aws_lc_rs::default_provider()
        .kx_groups
        .iter()
        .map(|group| group.name())
        .collect();
    assert_eq!(offered, defaults);
}
