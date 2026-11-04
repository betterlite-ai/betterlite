use std::collections::BTreeMap;
use std::fs;

use rstest::rstest;

use super::*;
use crate::tls::TlsError;

#[rstest]
#[case::small_invalid_pem(8, false)]
#[case::second_size_boundary(10_000, false)]
#[case::first_size_boundary(2 * 1024 * 1024, false)]
#[case::maximum_size(4 * 1024 * 1024, false)]
#[case::over_maximum(4 * 1024 * 1024 + 1, true)]
fn http_policy_tests_behavior(#[case] size: usize, #[case] oversized: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ca.pem");
    fs::write(&path, vec![b'x'; size]).unwrap();
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    let path_text = path.to_str().unwrap().to_owned();
    let environment = BTreeMap::from([("SSL_CERT_FILE".to_owned(), path_text)]);
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let result = apply_policy(ClientBuilder::new(), &policy);
    if oversized {
        assert!(matches!(result, Err(TlsError::CaTooLarge)));
    } else {
        assert!(matches!(
            result,
            Err(TlsError::InvalidCa(_) | TlsError::EmptyCa)
        ));
    }
}
