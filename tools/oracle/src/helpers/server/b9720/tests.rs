#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use serde_json::{Value, json};

use super::{Input, Op};

#[derive(serde::Deserialize)]
struct SeedCall {
    call: BTreeMap<String, Value>,
}

fn decode(input: Value) -> Input {
    serde_json::from_value(input).expect("the typed helper input")
}

fn answer(input: Value) -> Value {
    <Op as Helper>::run(&decode(input)).expect("the helper answers")
}

fn seal_of(writes: Value) -> Value {
    answer(json!({ "kind": "seal", "writes": writes }))
}

#[test]
fn the_seal_folds_the_start_tail_over_each_write() {
    let shaped = seal_of(json!(["data: {\"x\": 1}\n\n", "data: {"]));
    assert_eq!(
        shaped,
        json!({
            "tails": ["0a0a", "0a0a", "613a207b"],
            "seals": [
                "",
                "",
                "\nevent: ping\ndata: {\"type\": \"ping\"}\n\n",
            ],
        })
    );
}

#[test]
fn a_delimiter_split_across_writes_seals_empty() {
    let shaped = seal_of(json!(["data: x\n", "\n"]));
    assert_eq!(
        shaped,
        json!({
            "tails": ["0a0a", "3a20780a", "20780a0a"],
            "seals": [
                "",
                "event: ping\ndata: {\"type\": \"ping\"}\n\n",
                "",
            ],
        })
    );
}

#[test]
fn an_empty_write_keeps_the_tail() {
    let shaped = seal_of(json!(["", "data: {\"x\": 1}\n\n", ""]));
    assert_eq!(
        shaped,
        json!({
            "tails": ["0a0a", "0a0a", "0a0a", "0a0a"],
            "seals": ["", "", "", ""],
        })
    );
}

#[test]
fn a_multibyte_tail_is_the_last_four_written_bytes() {
    let shaped = seal_of(json!(["ééé"]));
    assert_eq!(
        shaped,
        json!({
            "tails": ["0a0a", "c3a9c3a9"],
            "seals": [
                "",
                "\nevent: ping\ndata: {\"type\": \"ping\"}\n\n",
            ],
        })
    );
}

#[test]
fn the_raw_model_name_comes_from_a_true_bool_in_either_metadata() {
    assert_eq!(
        answer(json!({
            "kind": "raw_model",
            "request_data": {
                "model": "gpt-4o",
                "metadata": {"_complexity_router_return_raw_model_name": true},
            },
        })),
        json!({ "raw": true })
    );
    assert_eq!(
        answer(json!({
            "kind": "raw_model",
            "request_data": {
                "litellm_metadata": {"_complexity_router_return_raw_model_name": true},
            },
        })),
        json!({ "raw": true })
    );
}

#[test]
fn a_non_true_or_missing_flag_keeps_the_requested_model() {
    assert_eq!(
        answer(json!({
            "kind": "raw_model",
            "request_data": {"metadata": {"_complexity_router_return_raw_model_name": "true"}},
        })),
        json!({ "raw": false })
    );
    assert_eq!(
        answer(json!({ "kind": "raw_model", "request_data": {"model": "gpt-4o"} })),
        json!({ "raw": false })
    );
}

#[test]
fn a_failure_becomes_an_error_body_and_an_unsealed_frame() {
    let shaped = answer(json!({
        "kind": "failure",
        "failure": {"message": "slow down", "type": null, "param": null, "status_code": 429},
    }));
    assert_eq!(
        shaped,
        json!({
            "error": {
                "message": "slow down",
                "type": "rate_limit_error",
                "param": null,
                "code": "429",
            },
            "frame_status": 429,
            "frame_body": "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"rate_limit_error\", \"message\": \"slow down\"}}\n\n",
        })
    );
}

#[test]
fn the_router_code_remap_leaves_the_error_type_alone() {
    let shaped = answer(json!({
        "kind": "failure",
        "failure": {
            "message": "No deployments available for selected model",
            "type": null,
            "param": null,
            "status_code": 500,
        },
    }));
    assert_eq!(
        shaped,
        json!({
            "error": {
                "message": "No deployments available for selected model",
                "type": "internal_server_error",
                "param": null,
                "code": "429",
            },
            "frame_status": 429,
            "frame_body": "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"rate_limit_error\", \"message\": \"No deployments available for selected model\"}}\n\n",
        })
    );
}

#[test]
fn a_failure_without_a_status_is_a_500_api_error_frame() {
    let shaped = answer(json!({
        "kind": "failure",
        "failure": {"message": "gone", "type": null, "param": null, "status_code": null},
    }));
    assert_eq!(
        shaped,
        json!({
            "error": {
                "message": "gone",
                "type": "internal_server_error",
                "param": null,
                "code": "500",
            },
            "frame_status": 500,
            "frame_body": "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"api_error\", \"message\": \"gone\"}}\n\n",
        })
    );
}

#[test]
fn a_failure_message_reaches_the_client_redacted() {
    let shaped = answer(json!({
        "kind": "failure",
        "failure": {
            "message": "Bearer abcdefghij1234567890",
            "type": "RateLimitError",
            "param": "messages",
            "status_code": 400,
        },
    }));
    assert_eq!(
        shaped,
        json!({
            "error": {
                "message": "REDACTED",
                "type": "RateLimitError",
                "param": "messages",
                "code": "400",
            },
            "frame_status": 400,
            "frame_body": "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"invalid_request_error\", \"message\": \"REDACTED\"}}\n\n",
        })
    );
}

#[test]
fn a_missing_payload_is_a_value_error() {
    let failure = <Op as Helper>::run(&decode(json!({ "kind": "raw_model" })))
        .expect_err("request_data is required");
    assert_eq!(failure.class, "ValueError");
    assert_eq!(failure.message, "the b9720 request_data field is required");
}

#[rstest]
fn replays_the_shared_stream_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/shared-stream/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}

#[test]
fn replays_the_b9720_helper_cases() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("b9720-"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert!(!seeds.is_empty(), "no b9720 seed exists");
    assert!(seeds.iter().all(|source| {
        let named = serde_saphyr::from_str::<SeedCall>(source).expect("a seed parses");
        let case = serde_saphyr::from_str::<HelperCase>(source).expect("a seed parses");
        validate_helper(Path::new("<helper>"), &case).expect("a seed validates");
        named.call.contains_key("b9720")
    }));
}
