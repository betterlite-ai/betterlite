#![cfg(test)]

use super::{WordsError, decode, load};
use std::{fs, io::ErrorKind, path::Path};

use rstest::rstest;

#[rstest]
#[case::valid("blocked_words:\n- keyword: SECRET\n  action: MASK\n  description: hidden\n", Some(vec!["secret"]))]
#[case::invalid_entries("blocked_words:\n- keyword: bad\n- action: BLOCK\n- 42\n- text\n- []\n- null\n- keyword: GOOD\n  action: BLOCK", Some(vec!["good"]))]
#[case::duplicates("blocked_words:\n- {keyword: SAME, action: MASK}\n- {keyword: same, action: BLOCK}", Some(vec!["same"]))]
#[case::missing_key("other: []", None)]
#[case::wrong_structure("[]", None)]
#[case::invalid_action("blocked_words: [{keyword: x, action: invalid}]", None)]
#[case::malformed("[", None)]
#[case::empty("", None)]
fn pattern_matching_behavior(#[case] yaml: &str, #[case] expected: Option<Vec<&str>>) {
    let words = decode(yaml.as_bytes());
    if let Some(names) = expected {
        assert_eq!(
            words
                .unwrap()
                .iter()
                .map(|word| word.keyword.as_str())
                .collect::<Vec<_>>(),
            names
        );
    } else {
        words.unwrap_err();
    }
}

#[test]
fn missing_structure_has_exact_message() {
    assert_eq!(
        decode(b"other: []").unwrap_err().to_string(),
        "Invalid format: file must contain 'blocked_words' key with list of words"
    );
}

#[test]
fn missing_file_keeps_not_found_cause() {
    let root = tempfile::tempdir().unwrap();
    let error = load(&root.path().join("missing.yaml")).unwrap_err();
    assert!(
        matches!(error, WordsError::Read { source, .. } if source.kind() == ErrorKind::NotFound)
    );
}

#[test]
fn replays_the_blocked_words_yaml_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/blocked_words_yaml");
    let outcomes: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            decode(&bytes)
        })
        .collect();
    assert!(!outcomes.is_empty());
}

#[path = "tests/warnings/tests.rs"]
mod warnings;

#[path = "tests/file/tests.rs"]
mod file;
