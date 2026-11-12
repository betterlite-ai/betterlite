#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

const SECTIONS: [&str; 4] = [
    "batch_requests",
    "embed_content_requests",
    "batch_replies",
    "embed_content_replies",
];

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_embedding_records"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn seed_cases() -> Vec<HelperCase> {
    seed_sources()
        .iter()
        .map(|source| serde_saphyr::from_str(source).unwrap())
        .collect()
}

#[test]
fn helper_seeds_select_gemini_embedding_records() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    assert!(sources.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "gemini_embedding_records"
    }));
}

#[test]
fn helper_seeds_cover_every_section() {
    let cases = seed_cases();
    let filled = SECTIONS.map(|section| {
        cases.iter().any(|case| {
            case.call
                .input
                .as_object()
                .and_then(|input| input.get(section))
                .and_then(|value| value.as_array())
                .is_some_and(|entries| !entries.is_empty())
        })
    });
    assert!(filled.iter().all(|present| *present));
}
