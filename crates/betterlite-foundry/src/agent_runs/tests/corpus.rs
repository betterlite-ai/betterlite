#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::agent_runs::decode::{
    decode_agent_messages, decode_agent_run_status, decode_created_id,
};

fn mismatch(name: &str, bytes: &[u8]) -> bool {
    let expected_ok = name.starts_with("ok-");
    match name.split('-').nth(1) {
        Some("status") => decode_agent_run_status(bytes).is_ok() != expected_ok,
        Some("created") => decode_created_id(bytes).is_ok() != expected_ok,
        Some("messages") => decode_agent_messages(bytes).is_ok() != expected_ok,
        Some("bytes") => {
            !expected_ok
                && (decode_agent_run_status(bytes).is_ok()
                    || decode_created_id(bytes).is_ok()
                    || decode_agent_messages(bytes).is_ok())
        }
        Some(_) | None => true,
    }
}

#[test]
fn replays_the_foundry_agent_runs_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_agent_runs");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty(), "the corpus folder holds seeds");
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .expect("corpus file name");
            mismatch(name, &bytes).then(|| name.to_owned())
        })
        .collect();
    assert!(
        failures.is_empty(),
        "seeds with the wrong outcome: {failures:?}"
    );
}
