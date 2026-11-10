#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

#[rstest]
#[case("azure_certificate-pem-seed")]
#[case("azure_certificate-pkcs12-seed")]
#[case("azure_certificate-denied-seed")]
fn helper_seeds_select_azure_certificate(#[case] seed: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/helper_case")
        .join(format!("{seed}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    assert_eq!(case.call.operation, "azure_certificate");
}
