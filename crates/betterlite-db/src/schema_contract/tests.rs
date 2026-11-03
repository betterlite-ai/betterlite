#![cfg(test)]
use rstest::rstest;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::Path;

#[rstest]
#[case::root("root")]
#[case::proxy("proxy")]
#[case::extras("extras")]
fn schema_fixture_hashes(#[case] directory: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/schema_contract/fixtures")
        .join(directory)
        .join("schema.prisma");
    let bytes = fs::read(path).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        "a17e69687d14d78dc7cee49770f1c1e8b2726cd8823d37c6dff48ae151533342"
    );
    assert_eq!(String::from_utf8(bytes).unwrap().lines().count(), 1917);
}

include!("tests/harness.rs");
include!("tests/catalog.rs");
include!("tests/mutations.rs");
include!("tests/semantics.rs");
include!("tests/isolation.rs");
include!("tests/physical.rs");
