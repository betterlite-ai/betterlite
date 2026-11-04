use std::collections::BTreeMap;

use rstest::rstest;

use super::*;

#[rstest]
#[case::x25519("X25519")]
#[case::prime256v1("prime256v1")]
#[case::secp384r1("secp384r1")]
#[case::invalid_falls_back("invalid")]
fn http_policy_tests_behavior(#[case] curve: &str) {
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
