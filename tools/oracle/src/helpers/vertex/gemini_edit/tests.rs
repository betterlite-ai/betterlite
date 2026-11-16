#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::helpers::Helper as _;

use super::Op;
use super::ops::Input;

const PNG: &str = "iVBORw0KGgo=";
const DIGITS: &str = "MDEyMzQ1Njc4OWFiY2RlZg==";

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_gemini_edit"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

fn decode(source: &str) -> Input {
    let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
    assert_eq!(case.call.operation, "vertex_gemini_edit");
    serde_json::from_value(case.call.input).expect("seed input should decode")
}

fn input(value: Value) -> Input {
    serde_json::from_value(value).expect("the helper input should decode")
}

fn request(image: Value, prompt: Option<&str>, size: Option<&str>) -> Input {
    let optional_params = match size {
        Some(size) => json!({"size": size}),
        None => json!({}),
    };
    input(json!({
        "model": "gemini-2.5-flash-image",
        "prompt": prompt,
        "image": image,
        "optional_params": optional_params,
        "target": null,
        "response": null,
    }))
}

#[test]
fn helper_seeds_select_vertex_gemini_edit() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let inputs: Vec<Input> = sources.iter().map(|source| decode(source)).collect();
    assert!(inputs.iter().any(|input| input.response.is_some()));
    assert!(inputs.iter().any(|input| input.image.is_some()));
}

#[test]
fn a_custom_base_is_trimmed_and_skips_authentication() {
    let parsed = Op::run(&input(json!({
        "model": "gemini-2.5-flash-image",
        "prompt": "make it blue",
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "optional_params": {"size": "1792x1024"},
        "target": {
            "api_base": "https://proxy.example/v1beta///",
            "params_api_base": null,
            "headers": {"x-goog-user-project": "billing"},
        },
        "response": null,
    })))
    .expect("a custom base needs no token");
    assert_eq!(parsed["supported"], json!(["size"]));
    assert_eq!(parsed["mapped"], json!({"aspectRatio": "16:9"}));
    assert_eq!(parsed["files"], json!(0));
    assert_eq!(parsed["positions"], json!([null]));
    assert_eq!(
        parsed["target"]["url"],
        json!("https://proxy.example/v1beta")
    );
    assert_eq!(
        parsed["target"]["headers"],
        json!({"x-goog-user-project": "billing"})
    );
    assert_eq!(parsed["payload"]["contents"]["role"], json!("USER"));
    assert_eq!(
        parsed["payload"]["contents"]["parts"],
        json!([
            {"inlineData": {"mimeType": "image/png", "data": PNG}},
            {"text": "make it blue"},
        ])
    );
    assert_eq!(
        parsed["payload"]["generationConfig"],
        json!({"response_modalities": ["IMAGE"], "image_config": {"aspect_ratio": "16:9"}})
    );
}

#[test]
fn a_seekable_record_is_read_from_zero_and_keeps_its_position() {
    let parsed = Op::run(&request(
        json!([{"kind": "seekable", "data_base64": DIGITS, "position": 7}]),
        None,
        None,
    ))
    .expect("the stream reads");
    assert_eq!(parsed["positions"], json!([7]));
    assert_eq!(
        parsed["payload"]["contents"]["parts"][0]["inlineData"]["data"],
        json!(DIGITS)
    );
}

#[rstest]
#[case::empty_list(json!([]))]
#[case::all_none(json!([null]))]
#[case::absent(Value::Null)]
fn an_image_list_without_a_part_is_a_value_error(#[case] image: Value) {
    let error = Op::run(&request(image, None, None)).expect_err("no part is refused");
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
}

#[test]
fn an_unsupported_record_is_a_value_error() {
    let error = Op::run(&request(json!([{"kind": "unsupported"}]), None, None))
        .expect_err("the record carries no bytes");
    assert_eq!(error.class, "ValueError");
}

#[test]
fn an_empty_inline_data_value_is_dropped() {
    let parsed = Op::run(&input(json!({
        "model": "gemini-2.5-flash-image",
        "prompt": null,
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "optional_params": {},
        "target": null,
        "response": {
            "candidates": [
                {"content": {"parts": [
                    {"inlineData": {"data": ""}},
                    {"inlineData": {"data": "AAA"}},
                    {"text": "ignored"},
                ]}},
                {"finishReason": "STOP"},
            ],
            "usageMetadata": {"totalTokenCount": 9},
        },
    })))
    .expect("the reply transforms");
    assert_eq!(parsed["response"], json!({"data": [{"b64_json": "AAA"}]}));
}

#[test]
fn a_reply_without_candidates_has_no_data() {
    let parsed = Op::run(&input(json!({
        "model": "gemini-2.5-flash-image",
        "prompt": null,
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "optional_params": {},
        "target": null,
        "response": {},
    })))
    .expect("an absent candidate list is no data");
    assert_eq!(parsed["response"], json!({"data": []}));
}
