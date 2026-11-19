#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use betterlite_openai::search::records::SearchResponse;

use crate::agentcore_search_reply::decode_agentcore_search_reply;

const SEED_STATUS: u16 = 200;

#[test]
fn replays_the_agentcore_search_reply_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/agentcore_search_reply");
    let mut seeds = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let outcome = decode_agentcore_search_reply(&bytes, SEED_STATUS, &[]);
            assert_eq!(outcome.is_ok(), expects_success(&name), "{name}");
            if let Ok(response) = outcome {
                check_results(&name, &response);
            }
            seeds = seeds.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seeds >= 12, "expected committed seeds, found {seeds}");
}

fn expects_success(name: &str) -> bool {
    name.starts_with("valid-") || name.starts_with("malformed-")
}

fn check_results(name: &str, response: &SearchResponse) {
    if name.contains("empty") {
        assert!(response.results.is_empty(), "{name}");
    }
    if name.contains("results") {
        assert!(!response.results.is_empty(), "{name}");
    }
}
