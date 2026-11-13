#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

const OPERATION: &str = "gemini_thinking";

#[test]
fn replays_the_gemini_thinking_seeds() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .expect("the helper corpus exists")
        .map(|entry| entry.expect("a corpus entry is readable").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(OPERATION))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("the seed is readable text");
            let parsed: HelperCase = serde_saphyr::from_str(&source).expect("the seed parses");
            validate_helper(Path::new("<helper>"), &parsed)
                .expect("the seed is a valid helper case");
            parsed
        })
        .collect();
    assert!(!seeds.is_empty(), "the corpus holds a {OPERATION} seed");
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == OPERATION
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}
