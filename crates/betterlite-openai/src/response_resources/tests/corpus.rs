use std::fs;
use std::path::{Path, PathBuf};

use super::super::error::ResponseResourceError;
use super::super::response::decode_response_resource;

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let decoded = decode_response_resource(&bytes);
    match name.split('-').next().unwrap() {
        "valid" | "boundary" => decoded.is_ok(),
        "malformed" | "truncated" => matches!(decoded, Err(ResponseResourceError::Decode { .. })),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_response_resources_corpus() {
    let folder =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_response_resources");
    let seeds: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(
        !seeds.is_empty(),
        "the openai_response_resources corpus is empty"
    );
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
