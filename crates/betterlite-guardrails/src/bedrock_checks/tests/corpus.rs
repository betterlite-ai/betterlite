#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::bedrock_records::ChecksResponse;

#[test]
fn replays_the_bedrock_checks_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_checks_response");
    let entries: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!entries.is_empty());
    let decoded: Vec<ChecksResponse> = entries
        .into_iter()
        .filter_map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            serde_json::from_slice(&bytes).ok()
        })
        .collect();
    assert!(decoded.iter().any(|response| {
        response
            .results
            .as_ref()
            .and_then(|results| results.sensitive_information.as_ref())
            .is_some_and(|sensitive| sensitive.truncated == Some(true))
    }));
    assert!(decoded.iter().any(|response| {
        response
            .results
            .as_ref()
            .and_then(|results| results.prompt_attack.as_ref())
            .is_some_and(|attack| attack.results.is_some())
    }));
}
