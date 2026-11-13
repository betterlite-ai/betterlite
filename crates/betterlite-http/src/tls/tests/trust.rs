use crate::tls::trust_roots;
use rstest::rstest;

#[rstest]
#[case::bundled_ca()]
fn http_policy_tests_behavior() {
    let roots = trust_roots(None).unwrap();
    assert_eq!(roots.roots, webpki_roots::TLS_SERVER_ROOTS);
}
