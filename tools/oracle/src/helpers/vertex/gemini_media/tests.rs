#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;

#[test]
fn helper_seeds_select_gemini_media() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_media"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let checked: Vec<(String, bool)> = seeds
        .iter()
        .map(|seed| {
            let nonempty = seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty());
            (seed.call.operation.clone(), nonempty)
        })
        .collect();
    assert!(
        checked
            .iter()
            .all(|(operation, nonempty)| operation == "gemini_media" && *nonempty)
    );
}
