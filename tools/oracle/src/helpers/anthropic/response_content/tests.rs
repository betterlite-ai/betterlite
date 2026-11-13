#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;

#[test]
fn helper_seeds_select_anthropic_response_content() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds = fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("anthropic_response_content"))
        })
        .collect::<Vec<_>>();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|source| serde_saphyr::from_str::<HelperCase>(&source).ok())
            .is_some_and(|case| case.call.operation == "anthropic_response_content")
    }));
}
