#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;
use super::ops::Input;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_image_policy"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

fn decode(source: &str) -> Input {
    let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
    assert_eq!(case.call.operation, "vertex_image_policy");
    serde_json::from_value(case.call.input).expect("seed input should decode")
}

fn input(billing: &Value, replies: &Value) -> Input {
    serde_json::from_value(json!({"models": [], "billing": billing, "replies": replies}))
        .expect("the helper input should decode")
}

fn billing(kind: &str, response: &Value) -> Value {
    json!([{
        "kind": kind,
        "model": "oracle-vertex-image",
        "catalog": {
            "output_cost_per_image": 0.5,
            "input_cost_per_token": 0_i64,
            "output_cost_per_token": 0_i64,
        },
        "response": response,
    }])
}

#[test]
fn helper_seeds_select_vertex_image_policy() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let inputs: Vec<Input> = sources.iter().map(|source| decode(source)).collect();
    assert!(inputs.iter().any(|input| !input.models.is_empty()));
    assert!(inputs.iter().any(|input| !input.replies.is_empty()));
}

#[test]
fn models_name_the_python_config_classes() {
    let input: Input = serde_json::from_value(json!({
        "models": ["gemini-2.5-flash-image", "imagegeneration@006"],
        "billing": [],
        "replies": [],
    }))
    .expect("the helper input should decode");
    let value = Op::run(&input).expect("selection never fails");
    assert_eq!(
        value["models"][0]["generation"],
        json!("VertexAIGeminiImageGenerationConfig")
    );
    assert_eq!(
        value["models"][0]["edit"],
        json!("VertexAIGeminiImageEditConfig")
    );
    assert_eq!(
        value["models"][1]["generation"],
        json!("VertexAIImagenImageGenerationConfig")
    );
    assert_eq!(
        value["models"][1]["edit"],
        json!("VertexAIImagenImageEditConfig")
    );
}

#[test]
fn a_zero_token_price_costs_nothing_even_with_a_per_image_price() {
    let usage = json!({
        "input_tokens": 10_i64,
        "output_tokens": 20_i64,
        "total_tokens": 30_i64,
        "web_search_requests": null,
    });
    let input = input(
        &billing("generation", &json!({"images": 3_i64, "usage": usage})),
        &json!([]),
    );
    let value = Op::run(&input).expect("the cost is priced");
    assert_eq!(value["billing"][0]["cost"], json!("0"));
}

#[test]
fn search_alone_is_priced_when_no_image_came_back() {
    let usage = json!({
        "input_tokens": 0_i64,
        "output_tokens": 0_i64,
        "total_tokens": 0_i64,
        "web_search_requests": 2_i64,
    });
    let input = input(
        &billing("generation", &json!({"images": 0_i64, "usage": usage})),
        &json!([]),
    );
    let value = Op::run(&input).expect("the cost is priced");
    assert_eq!(value["billing"][0]["cost"], json!("0.035"));
}

#[test]
fn an_edit_response_that_is_not_an_image_response_fails_as_a_value_error() {
    let input = input(&billing("edit", &Value::Null), &json!([]));
    let error = Op::run(&input).expect_err("not an ImageResponse");
    assert_eq!(error.class, "ValueError");
}

#[test]
fn a_reply_without_predictions_fails_as_an_internal_server_error() {
    let replies = json!([{"kind": "convert", "body": {}, "existing": ["OLD"]}]);
    let error = Op::run(&input(&json!([]), &replies)).expect_err("no predictions");
    assert_eq!(error.class, "InternalServerError");
    assert_eq!(error.status, Some(500));
}

#[test]
fn a_conversion_replaces_the_images_the_response_held() {
    let replies = json!([{
        "kind": "convert",
        "body": {"predictions": [{"bytesBase64Encoded": "AAA"}]},
        "existing": ["OLD", null],
    }]);
    let value = Op::run(&input(&json!([]), &replies)).expect("the reply converts");
    assert_eq!(value["replies"][0]["images"], json!(["AAA"]));
}
