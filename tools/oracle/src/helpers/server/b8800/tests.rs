#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_oracle::case::helper::{HelperCase, validate_helper};
use serde_json::{Value, json};

use super::{Input, Op};
use crate::helpers::{Helper, HelperError};

#[derive(serde::Deserialize)]
struct SeedCall {
    call: BTreeMap<String, Value>,
}

fn decode(input: Value) -> Input {
    serde_json::from_value(input).expect("typed helper input")
}

fn answer(input: Value) -> Value {
    <Op as Helper>::run(&decode(input)).expect("the helper answers")
}

fn failure(input: Value) -> HelperError {
    *<Op as Helper>::run(&decode(input)).expect_err("the helper fails")
}

#[test]
fn stream_usage_updates_merges_the_client_options() {
    let value = answer(json!({
        "function": "stream_usage_updates",
        "data": {"stream": true, "stream_options": {"a": true, "include_usage": false}},
        "route_type": "acompletion",
        "supports": true
    }));
    assert_eq!(
        value,
        json!({
            "stream_options": {"a": true, "include_usage": true},
            "_litellm_strip_stream_usage": true
        })
    );
}

#[test]
fn stream_usage_updates_takes_the_always_include_merge() {
    let value = answer(json!({
        "function": "stream_usage_updates",
        "data": {"stream": true, "_litellm_strip_stream_usage": true},
        "always_include": true,
        "supports": false
    }));
    assert_eq!(
        value,
        json!({
            "_litellm_strip_stream_usage": false,
            "stream_options": {"include_usage": true}
        })
    );
}

#[test]
fn an_unknown_route_type_is_a_value_error() {
    let error = failure(json!({
        "function": "stream_usage_updates",
        "data": {},
        "route_type": "completions"
    }));
    assert_eq!((error.class.as_str(), error.status), ("ValueError", None));
}

#[test]
fn file_search_store_ids_are_sorted_and_deduplicated() {
    let value = answer(json!({
        "function": "file_search_store_ids",
        "data": {"tools": [
            {"type": "file_search", "vector_store_ids": ["b", "a"]},
            {"type": "file_search", "vector_store_ids": ["a", "c"]}
        ]}
    }));
    assert_eq!(value, json!(["a", "b", "c"]));
}

#[test]
fn a_bad_store_id_is_a_400() {
    let error = failure(json!({
        "function": "file_search_store_ids",
        "data": {"tools": [{"type": "file_search", "vector_store_ids": ["vs_1", true]}]}
    }));
    assert_eq!(
        (error.class.as_str(), error.status),
        ("HTTPException", Some(400))
    );
}

#[test]
fn request_model_answers_the_validated_model() {
    let mixed = answer(json!({
        "function": "request_model",
        "data": {"model": ["gpt-4o", true]}
    }));
    assert_eq!(mixed, Value::Null);
    let listed = answer(json!({
        "function": "request_model",
        "data": {"model": ["gpt-4o", "gpt-4o-mini"]}
    }));
    assert_eq!(listed, json!(["gpt-4o", "gpt-4o-mini"]));
}

#[test]
fn replays_the_b8800_helper_cases() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("b8800-"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let named = serde_saphyr::from_str::<SeedCall>(source).expect("seed should parse");
        let parsed = serde_saphyr::from_str::<HelperCase>(source).expect("seed should parse");
        validate_helper(Path::new("<helper>"), &parsed).expect("seed should validate");
        named.call.contains_key("b8800")
    }));
}
