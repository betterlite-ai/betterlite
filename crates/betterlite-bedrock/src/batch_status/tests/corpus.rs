#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::batch_status::decode::decode_batch_status;

#[test]
fn replays_the_bedrock_services_batch_status_corpus() {
    let seeds = replay_corpus();
    assert!(seeds >= 4, "the corpus holds only {seeds} seeds");
}

fn replay_corpus() -> usize {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_batch_status");
    let entries = fs::read_dir(corpus).expect("the corpus directory exists");
    let mut seeds = 0_usize;
    for entry in entries {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let expected = name.starts_with("valid") || name.starts_with("boundary-valid");
        assert_eq!(
            decode_batch_status(&bytes, 200, 1_700_000_000).is_ok(),
            expected,
            "seed {name}"
        );
        seeds = seeds.saturating_add(1);
    }
    seeds
}
