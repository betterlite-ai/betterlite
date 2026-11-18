#![cfg(test)]

use std::fs;
use std::path::Path;

use sha2::{Digest as _, Sha256};

pub(crate) fn litellm_statements() -> Vec<String> {
    let manifest = include_str!("fixtures/sql.sha256");
    assert_eq!(
        hex::encode(Sha256::digest(manifest)),
        "c2a1e005f73eda0bb5f5496f90299688872f1e56e71f749b52d81009eae0d5ee"
    );
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/schema_contract/fixtures/sql");
    manifest
        .lines()
        .map(|line| {
            let (digest, file) = line.split_once("  ").unwrap();
            let sql = fs::read(folder.join(file)).unwrap();
            assert_eq!(hex::encode(Sha256::digest(&sql)), digest);
            String::from_utf8(sql).unwrap()
        })
        .collect()
}
