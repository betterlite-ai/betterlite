#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json;

use crate::count_tokens::wire::decode_count_response;

#[test]
fn replays_the_anthropic_count_tokens_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_count_tokens");
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
    assert!(sources.len() >= 4);
    assert!(sources.iter().all(|(name, bytes)| {
        let decoded = decode_count_response(bytes);
        match name.as_str() {
            "valid.json" | "boundary.json" => decoded.is_ok(),
            "malformed.json" | "truncated.json" | "non-object.json" => decoded.is_err(),
            _ => true,
        }
    }));
    let (_, valid) = sources
        .iter()
        .find(|(name, _)| name == "valid.json")
        .unwrap();
    let decoded = decode_count_response(valid).unwrap();
    assert_eq!(
        decoded.get("input_tokens"),
        Some(&Json::Number(serde_json::Number::from(42_u8)))
    );
}
