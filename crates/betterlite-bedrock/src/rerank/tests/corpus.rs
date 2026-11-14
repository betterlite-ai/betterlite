#![cfg(test)]

use std::fs;
use std::path::Path;

use super::cases::GENERATED_ID;
use crate::rerank::decode::decode_rerank_reply;

#[test]
fn replays_the_bedrock_other_rerank_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_other_rerank");
    let sources = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    assert!(sources.len() >= 8, "the corpus carries every category");
    for (name, bytes) in &sources {
        let accepted = name.starts_with("valid") || name.starts_with("boundary");
        assert_eq!(
            decode_rerank_reply(bytes, GENERATED_ID).is_ok(),
            accepted,
            "{name}"
        );
    }
    let (_, bytes) = sources
        .iter()
        .find(|(name, _)| name == "valid-usage.json")
        .unwrap();
    let response = decode_rerank_reply(bytes, GENERATED_ID).unwrap();
    let meta = response.meta.as_ref().unwrap();
    assert_eq!(
        meta.billed_units
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .search_units,
        Some(Some(2))
    );
}
