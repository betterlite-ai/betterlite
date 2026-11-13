use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::super::error::ResponseResourceError;
use super::super::response::{MAX_BODY_BYTES, decode_response_resource, json_body};
use crate::response_events::RawResponse;

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

#[test]
fn decode_response_resource_refuses_oversized_inputs() {
    let too_many = serde_json::to_vec(&vec![0_u8; 0x4001]).unwrap();
    assert!(matches!(
        decode_response_resource(&too_many),
        Err(ResponseResourceError::Decode { .. })
    ));
    let too_large = "a".repeat(MAX_BODY_BYTES.saturating_add(1));
    assert!(matches!(
        decode_response_resource(too_large.as_bytes()),
        Err(ResponseResourceError::Decode { .. })
    ));
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: &too_large,
    };
    let error = json_body(&reply).unwrap_err();
    assert_eq!(error.http_status(), None);
}
