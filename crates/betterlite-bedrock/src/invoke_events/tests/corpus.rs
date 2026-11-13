#![cfg(test)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::invoke_events::framing::decode_bedrock_event_json;

#[test]
fn replays_the_bedrock_invoke_events_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_invoke_events");
    let paths: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!paths.is_empty(), "no bedrock_invoke_events seeds");
    paths
        .iter()
        .try_for_each(|path| {
            let bytes = fs::read(path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(
                decode_bedrock_event_json(&bytes).is_ok(),
                name.starts_with("valid-"),
                "{name}"
            );
            Ok::<(), io::Error>(())
        })
        .unwrap();
}
