#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::mongodb_vector_store::decode::decode_mongodb_search;

#[test]
fn replays_the_mongodb_vector_store_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mongodb_vector_store");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let failures: Vec<&str> = paths
        .iter()
        .filter(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("name");
            let decoded = decode_mongodb_search(&bytes);
            decoded.is_ok() != name.starts_with("ok-")
        })
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("name")
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
