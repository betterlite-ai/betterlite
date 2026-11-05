#![cfg(test)]

use super::{Op, canonical_arguments};
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::json;
use std::fs;
use std::path::Path;

#[test]
fn helper_runs_the_compatible_line_stream() {
    let input = json!({
        "mode": "async",
        "events": [
            {"kind": "line", "line": "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0}}"},
            {"kind": "line", "line": "data: [DONE]"}
        ]
    });
    assert_eq!(
        Op::decode_and_run(&input).unwrap(),
        json!([
            {"kind":"chunk","value":{"text":"","tool_use":null,"is_finished":false,"finish_reason":"","usage":{"prompt_tokens":0_i32,"completion_tokens":0_i32,"total_tokens":0_i32},"index":0_u32}},
            {"kind":"end"}
        ])
    );
}

#[rstest]
#[case::sync("sync", "connection reset")]
#[case::async_mode("async", "Error receiving chunk from stream: connection reset")]
fn helper_records_a_receive_error_and_stops(#[case] mode: &str, #[case] expected: &str) {
    let input = json!({"mode":mode,"events":[
        {"kind":"receive_error","message":"connection reset"},
        {"kind":"line","line":"{\"choices\":[{\"delta\":{\"content\":\"after\"}}]}"}
    ]});
    assert_eq!(
        Op::decode_and_run(&input).unwrap(),
        json!([{"kind":"error","value":expected}])
    );
}

#[rstest]
#[case::object(r#"{"a":1}"#, r#"{"a":1}"#)]
#[case::spaced(r#"{"a": 1}"#, r#"{"a":1}"#)]
#[case::plain_text("not json", "not json")]
fn canonical_arguments_compacts_objects_only(#[case] arguments: &str, #[case] expected: &str) {
    assert_eq!(canonical_arguments(arguments), expected);
}

#[rstest]
#[case::sync("sync")]
#[case::async_mode("async")]
fn helper_exhaustion_is_not_a_blank_chunk(#[case] mode: &str) {
    let input = json!({"mode":mode,"events":[]});
    assert_eq!(Op::decode_and_run(&input).unwrap(), json!([{"kind":"end"}]));
    let blank = json!({"mode":mode,"events":[{"kind":"line","line":""}]});
    let result = Op::decode_and_run(&blank).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert_eq!(result[0]["kind"], "chunk");
}

#[test]
fn helper_seeds_select_compatible_line_chunk() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("compatible_line_chunk")
        })
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        validate_helper(path, &case).unwrap();
        assert_eq!(case.call.operation, "compatible_line_chunk");
        crate::helpers::invoke(&case.call).ok
    }));
}
