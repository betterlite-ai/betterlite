#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use serde_json::{Value, json};

use super::Op;
use super::ops::Input;

const PNG: &str = "iVBORw0KGgo=";
const RAW_ONE: &str = "cmF3LW9uZQ==";
const RAW_TWO: &str = "cmF3LXR3bw==";
const MASK: &str = "bWFzay1ieXRlcw==";

fn seed_cases() -> Vec<HelperCase> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_imagen_edit"))
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let source = fs::read_to_string(path).expect("seed should be readable");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("seed should parse");
            assert_eq!(case.call.operation, "vertex_imagen_edit");
            validate_helper(path, &case)
                .map_err(|error| error.to_string())
                .expect("seed should validate");
            case
        })
        .collect()
}

fn decode(case: &HelperCase) -> Input {
    serde_json::from_value(case.call.input.clone()).expect("seed input should decode")
}

fn input(value: Value) -> Input {
    serde_json::from_value(value).expect("the helper input should decode")
}

fn reference(parsed: &Value, index: usize) -> Value {
    parsed["payload"]["instances"][0]["referenceImages"][index].clone()
}

#[test]
fn helper_seeds_select_vertex_imagen_edit() {
    let cases = seed_cases();
    assert!(!cases.is_empty());
    let inputs: Vec<Input> = cases.iter().map(decode).collect();
    assert!(inputs.iter().any(|parsed| parsed.mask.is_some()));
    assert!(inputs.iter().any(|parsed| parsed.target.is_some()));
    assert!(inputs.iter().any(|parsed| parsed.response.is_some()));
    assert!(cases.iter().all(|case| matches!(
        run_case(&[crate::GROUP], case, false).expect("the seed should run"),
        HelperRun::Match
    )));
}

#[rstest]
#[case::mask_id_gap("mask-id-gap")]
#[case::empty_prompt("empty-prompt")]
#[case::ignored_size("ignored-size")]
#[case::custom_host_resource_path("custom-host-resource-path")]
#[case::tuple_filename_rejected("tuple-filename-rejected")]
#[case::base64_key_fallback("base64-key-fallback")]
#[case::depth_boundary("depth-boundary")]
#[case::empty_image_list_mask("empty-image-list-mask")]
#[case::base64_stops_at_pad("base64-stops-at-pad")]
#[case::reply_created_and_usage("reply-created-and-usage")]
fn the_edit_cases_match_litellm(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR).join(format!("cases/helpers/vertex-imagen-edit/{name}.yaml"));
    let source = fs::read_to_string(&path).expect("the case should be readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("the case should parse");
    validate_helper(&path, &case)
        .map_err(|error| error.to_string())
        .expect("the case should validate");
    assert_eq!(case.call.operation, "vertex_imagen_edit");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("the case should run"),
        HelperRun::Match
    ));
}

#[test]
fn a_mask_takes_the_reference_count_and_may_repeat_an_id() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "fill the sky",
        "image": [
            {"kind": "bytes", "data_base64": RAW_ONE},
            null,
            {"kind": "bytes", "data_base64": RAW_TWO},
        ],
        "mask": {"kind": "bytes", "data_base64": MASK},
        "optional_params": {"n": 2},
        "target": null,
        "response": null,
    })))
    .expect("the edit request builds");
    assert_eq!(parsed["mapped"], json!({"sampleCount": 2}));
    assert_eq!(parsed["files"], json!(0));
    assert_eq!(reference(&parsed, 0)["referenceId"], json!(1));
    assert_eq!(
        reference(&parsed, 0)["referenceImage"]["bytesBase64Encoded"],
        json!(RAW_ONE)
    );
    assert_eq!(reference(&parsed, 1)["referenceId"], json!(3));
    assert_eq!(
        reference(&parsed, 1)["referenceImage"]["bytesBase64Encoded"],
        json!(RAW_TWO)
    );
    assert_eq!(
        reference(&parsed, 2)["referenceType"],
        json!("REFERENCE_TYPE_MASK")
    );
    assert_eq!(reference(&parsed, 2)["referenceId"], json!(3));
    assert_eq!(
        reference(&parsed, 2)["maskImageConfig"],
        json!({"maskMode": "MASK_MODE_USER_PROVIDED", "dilation": 0.03})
    );
    assert_eq!(parsed["payload"]["parameters"]["sampleCount"], json!(2));
}

#[test]
fn an_empty_image_list_sends_the_mask_alone() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "fill the sky",
        "image": [],
        "mask": {"kind": "bytes", "data_base64": MASK},
        "optional_params": {},
        "target": null,
        "response": null,
    })))
    .expect("a mask alone is a request");
    assert_eq!(
        reference(&parsed, 0)["referenceType"],
        json!("REFERENCE_TYPE_MASK")
    );
    assert_eq!(reference(&parsed, 0)["referenceId"], json!(1));
    let absent = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "fill the sky",
        "mask": {"kind": "bytes", "data_base64": MASK},
        "optional_params": {},
        "target": null,
        "response": null,
    })))
    .expect_err("an absent image is no request");
    assert_eq!(absent.class, "ValueError");
}

#[test]
fn a_pad_run_that_completes_a_quad_ends_the_base64_text() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [
            {"kind": "mapping", "data": "QQ==QUJDRA==", "bytes": null, "content": null, "path": null},
            {"kind": "mapping", "data": "QUI=QUJD", "bytes": null, "content": null, "path": null},
        ],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": null,
    })))
    .expect("the text before the pad run answers");
    assert_eq!(
        reference(&parsed, 0)["referenceImage"]["bytesBase64Encoded"],
        json!("QQ==")
    );
    assert_eq!(
        reference(&parsed, 1)["referenceImage"]["bytesBase64Encoded"],
        json!("QUI=")
    );
}

#[test]
fn an_empty_prompt_reaches_the_body() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "",
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": null,
    })))
    .expect("the edit request builds");
    assert_eq!(parsed["payload"]["instances"][0]["prompt"], json!(""));
    assert_eq!(parsed["payload"]["parameters"]["sampleCount"], json!(1));
}

#[test]
fn a_mapped_aspect_ratio_stays_out_of_the_body() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "mask": null,
        "optional_params": {"size": "1792x1024", "n": 3},
        "target": null,
        "response": null,
    })))
    .expect("the edit request builds");
    assert_eq!(
        parsed["mapped"],
        json!({"sampleCount": 3, "aspectRatio": "16:9"})
    );
    assert!(parsed["payload"]["parameters"]["aspectRatio"].is_null());
    assert_eq!(parsed["payload"]["parameters"]["sampleCount"], json!(3));
}

#[test]
fn a_custom_base_keeps_the_resource_path_and_the_headers() {
    let parsed = Op::run(&input(json!({
        "model": "vertex_ai/imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "mask": null,
        "optional_params": {},
        "target": {
            "api_base": "https://proxy.example/vertex/",
            "params_api_base": null,
            "project": "demo-project",
            "location": "us-central1",
            "headers": {"x-goog-user-project": "billing", "x-request-id": "req-1"},
        },
        "response": null,
    })))
    .expect("a custom base needs no token");
    assert_eq!(
        parsed["target"],
        json!({
            "url": "https://proxy.example/vertex/v1/projects/demo-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict",
            "headers": {"x-goog-user-project": "billing", "x-request-id": "req-1"},
        })
    );
}

#[test]
fn a_sequence_whose_first_member_is_a_file_name_is_refused() {
    let error = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{
            "kind": "sequence",
            "items": [
                {"kind": "text", "text": "image.png"},
                {"kind": "bytes", "data_base64": PNG},
            ],
        }],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": null,
    })))
    .expect_err("a file name is not image bytes");
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
}

#[test]
fn a_value_key_that_is_not_base64_falls_through_to_the_next_one() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{
            "kind": "mapping",
            "data": "not base64!",
            "bytes": PNG,
            "content": null,
            "path": null,
        }],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": null,
    })))
    .expect("the next key answers");
    assert_eq!(
        reference(&parsed, 0)["referenceImage"]["bytesBase64Encoded"],
        json!(PNG)
    );
}

#[test]
fn the_depth_limit_refuses_the_third_nested_list() {
    let error = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{
            "kind": "sequence",
            "items": [{
                "kind": "sequence",
                "items": [{
                    "kind": "sequence",
                    "items": [{"kind": "bytes", "data_base64": PNG}],
                }],
            }],
        }],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": null,
        "max_recurse_depth": 2,
    })))
    .expect_err("the reader stops past the limit");
    assert_eq!(error.class, "ValueError");
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{
            "kind": "sequence",
            "items": [{
                "kind": "sequence",
                "items": [{"kind": "bytes", "data_base64": PNG}],
            }],
        }],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": null,
        "max_recurse_depth": 2,
    })))
    .expect("two nested lists are read");
    assert_eq!(
        reference(&parsed, 0)["referenceImage"]["bytesBase64Encoded"],
        json!(PNG)
    );
}

#[test]
fn a_prediction_with_the_bytes_key_becomes_an_image() {
    let parsed = Op::run(&input(json!({
        "model": "imagen-3.0-capability-001",
        "prompt": "paint",
        "image": [{"kind": "bytes", "data_base64": PNG}],
        "mask": null,
        "optional_params": {},
        "target": null,
        "response": {
            "predictions": [
                {"bytesBase64Encoded": "AAA", "prompt": "ignored"},
                {"raiFilteredReason": "blocked"},
            ],
        },
    })))
    .expect("the reply transforms");
    assert_eq!(
        parsed["response"],
        json!({
            "created": 0,
            "data": [{"b64_json": "AAA"}],
            "usage": {
                "input_tokens": 0,
                "input_tokens_details": {"image_tokens": 0, "text_tokens": 0},
                "output_tokens": 0,
                "total_tokens": 0,
            },
        })
    );
}
