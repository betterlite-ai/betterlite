use std::fs;
use std::path::{Path, PathBuf};

use crate::workload_identity::error::WorkloadIdentityError;
use crate::workload_identity::exchange::decode_token_exchange;

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let decoded = decode_token_exchange(&bytes);
    match name.split('-').next().unwrap() {
        "valid" | "boundary" => decoded.is_ok(),
        "invalid" => matches!(decoded, Err(WorkloadIdentityError::InvalidField { .. })),
        "malformed" | "truncated" => matches!(decoded, Err(WorkloadIdentityError::EmptyBody)),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_workload_identity_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_workload_identity");
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!seeds.is_empty());
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
