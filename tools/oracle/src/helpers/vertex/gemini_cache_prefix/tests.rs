#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;

use super::ops::Input;

#[test]
fn helper_seeds_select_gemini_cache_prefix() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_cache_prefix"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(
        seeds
            .iter()
            .all(|seed| seed.call.operation == "gemini_cache_prefix")
    );
    let decoded: Vec<Input> = seeds
        .iter()
        .map(|seed| serde_json::from_value(seed.call.input.clone()).unwrap())
        .collect();
    assert_eq!(decoded.len(), seeds.len());
}
