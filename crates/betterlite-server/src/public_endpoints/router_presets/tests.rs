#![cfg(test)]

mod catalog;
mod concurrent;
mod fallback;
mod fetch;
mod fuse;
mod routes;
mod scorer;
mod support;

use std::fs;
use std::path::PathBuf;

use super::catalog::decode_catalog;

#[test]
fn replays_autorouter_presets_corpus() {
    let seeds = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/autorouter_presets");
    let replayed = fs::read_dir(seeds)
        .expect("the corpus directory should exist")
        .map(|entry| {
            let path = entry.expect("a corpus entry should be readable").path();
            let bytes = fs::read(&path).expect("a corpus seed should be readable");
            let first = decode_catalog(&bytes).map_err(|error| error.to_string());
            let second = decode_catalog(&bytes).map_err(|error| error.to_string());
            assert_eq!(first, second, "{}", path.display());
            let valid_seed = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("valid-"));
            assert_eq!(first.is_ok(), valid_seed, "{}", path.display());
        })
        .count();
    assert!(replayed >= 8, "replayed {replayed} seeds");
}
