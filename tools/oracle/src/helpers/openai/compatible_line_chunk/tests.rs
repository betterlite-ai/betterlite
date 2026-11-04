#![cfg(test)]

use super::{Op, Recorded, StreamEvent, StreamInput, run_stream};
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;
use betterlite_openai::compatible_line_stream::{Mode, Outcome};
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

#[test]
fn compatible_line_stream_stops_after_async_done_and_emits_exhaustion() {
    let input = StreamInput {
        mode: Mode::Async,
        events: vec![
            StreamEvent::Line {
                line: String::from(r#"{"choices":[{"delta":{"content":"before"}}]}"#),
            },
            StreamEvent::Line {
                line: String::from("data: [DONE]"),
            },
            StreamEvent::Line {
                line: String::from(r#"{"choices":[{"delta":{"content":"after"}}]}"#),
            },
        ],
    };
    let outcomes = run_stream(&input);
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes.last(), Some(&Recorded::Line(Outcome::End)));
}

#[rstest]
#[case::sync(Mode::Sync, "connection reset")]
#[case::async_receive(Mode::Async, "Error receiving chunk from stream: connection reset")]
fn compatible_line_stream_stops_after_receive_error(#[case] mode: Mode, #[case] expected: &str) {
    let input = StreamInput {
        mode,
        events: vec![
            StreamEvent::ReceiveError {
                message: String::from("connection reset"),
            },
            StreamEvent::Line {
                line: String::from(r#"{"choices":[{"delta":{"content":"after"}}]}"#),
            },
        ],
    };
    assert_eq!(
        run_stream(&input),
        vec![Recorded::Error {
            kind: "error",
            value: expected.to_owned()
        }]
    );
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
