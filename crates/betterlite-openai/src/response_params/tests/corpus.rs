use std::fs;
use std::path::{Path, PathBuf};

use super::super::decode::{ResponseDecodeError, decode_response_capability_error};

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let decoded = decode_response_capability_error(&bytes);
    match name.split('-').next().unwrap() {
        "valid" | "boundary" => decoded.is_ok(),
        "malformed" | "truncated" => matches!(decoded, Err(ResponseDecodeError::Json { .. })),
        "notobject" => matches!(decoded, Err(ResponseDecodeError::NotObject)),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_response_params_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_response_params");
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(
        !seeds.is_empty(),
        "the openai_response_params corpus is empty"
    );
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
