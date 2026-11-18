#![cfg(test)]

mod decode;
mod duration;
mod error_fields;
mod failure;
mod python_text;
mod state;
mod support;
mod tasks;

use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_stream_primitives::decode::{JsonShape, decode_responses_stream_value};

// the same seeds the responses_stream_value fuzz target mutates
#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_stream_value");
    let seeds: Vec<Vec<u8>> = fs::read_dir(corpus)
        .expect("the corpus folder")
        .map(|entry| fs::read(entry.expect("an entry").path()).expect("a seed file"))
        .collect();
    assert!(!seeds.is_empty());
    let decoded: Vec<Json> = seeds
        .iter()
        .filter_map(|bytes| decode_responses_stream_value(bytes, &ApiLimits::default()).ok())
        .collect();
    assert!(!decoded.is_empty());
    assert!(decoded.iter().any(JsonShape::is_json_object));
    assert!(decoded.iter().any(JsonShape::is_json_array));
    assert!(decoded.iter().any(|value| matches!(
        value,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)
    )));
}
