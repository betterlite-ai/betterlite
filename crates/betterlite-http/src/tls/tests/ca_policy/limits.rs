use crate::tls::{TlsError, parse_ca_pem};
use rstest::rstest;

#[rstest]
#[case::below_limit(4 * 1024 * 1024 - 1, false)]
#[case::at_limit(4 * 1024 * 1024, false)]
#[case::over_limit(4 * 1024 * 1024 + 1, true)]
fn http_policy_tests_behavior(#[case] length: usize, #[case] oversized: bool) {
    let mut bytes = include_bytes!("../../../../fuzz/corpus/http_policy/trusted-ca.pem").to_vec();
    bytes.resize(length, b' ');
    let result = parse_ca_pem(&bytes);
    if oversized {
        assert!(matches!(result, Err(TlsError::CaTooLarge)));
    } else {
        assert_eq!(result.unwrap().len(), 1);
    }
}
