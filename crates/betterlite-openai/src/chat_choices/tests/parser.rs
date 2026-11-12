#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use super::support::nil_id;
use crate::chat_choices::{ChatChoicesError, decode_chat_choices};

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let decoded = decode_chat_choices(&bytes, None, false, nil_id);
    match name.split('-').next().unwrap() {
        "valid" => decoded.is_ok(),
        "empty" => matches!(decoded, Ok(choices) if choices.is_empty()),
        "malformed" | "truncated" | "scalar" | "missing" | "boundary" => {
            matches!(decoded, Err(ChatChoicesError::Decode { .. }))
        }
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_chat_choices_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_chat_choices");
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!seeds.is_empty());
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
