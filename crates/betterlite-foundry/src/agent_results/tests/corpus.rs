#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::agent_results::event::decode_agent_event;

#[test]
fn replays_the_foundry_agent_results_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_agent_results");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .expect("name");
            (decode_agent_event(&bytes).is_ok() != name.starts_with("ok-")).then(|| name.to_owned())
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
