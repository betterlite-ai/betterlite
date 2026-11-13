#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

#[test]
fn helper_seeds_select_openai_transcription() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_transcription"))
        })
        .map(|path| fs::read_to_string(path).expect("the seed reads"))
        .collect();
    assert!(!seeds.is_empty(), "the transcription seeds exist");
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("the seed parses");
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "openai_transcription"
    }));
}
