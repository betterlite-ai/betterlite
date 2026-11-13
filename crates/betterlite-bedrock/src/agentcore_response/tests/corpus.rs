#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::agentcore_response::reply::decode_agentcore_reply;

#[test]
fn replays_the_bedrock_services_agentcore_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/bedrock_services_agentcore_response");
    let mut seeds = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let content_type = if name.contains("json") {
                Some("application/json")
            } else {
                Some("text/event-stream")
            };
            assert_eq!(
                decode_agentcore_reply(&bytes, content_type).is_ok(),
                name.starts_with("valid-"),
                "{name}"
            );
            seeds = seeds.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seeds >= 12, "expected committed seeds, found {seeds}");
}
