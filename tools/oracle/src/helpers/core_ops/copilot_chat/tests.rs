#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle::case::helper::{HelperCase, validate_helper};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Input, Op};
use crate::helpers::Helper as _;

#[derive(Deserialize)]
struct Call {
    copilot_chat: Input,
}

#[derive(Deserialize)]
struct Seed {
    call: Call,
}

fn seed_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fuzz/corpus/helper_case")
        .join(format!("copilot_chat_{name}"))
}

fn answer(name: &str) -> Value {
    let source = fs::read_to_string(seed_path(name)).expect("seed should be readable");
    let seed: Seed = serde_saphyr::from_str(&source).expect("seed should parse");
    Op::run(&seed.call.copilot_chat).expect("the helper should answer")
}

#[test]
fn every_copilot_chat_seed_is_a_valid_helper_case() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("copilot_chat_"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no copilot_chat seed exists");
    assert!(seeds.iter().all(|(name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        drop(
            serde_saphyr::from_str::<Seed>(source)
                .unwrap_or_else(|error| panic!("{name} must call copilot_chat: {error}")),
        );
        validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}

#[test]
fn system_role_switch_changes_only_the_role() {
    assert_eq!(
        answer("system_role_switch"),
        json!([
            {"role": "assistant", "content": "rules", "name": "setup"},
            {"role": "user", "content": "hi"},
            {"role": "developer", "content": "dev"}
        ])
    );
}

#[test]
fn parent_headers_win_over_the_copilot_defaults() {
    let headers = answer("parent_headers_win");
    assert_eq!(headers["Authorization"], "Bearer caller-key");
    assert_eq!(headers["editor-version"], "caller-editor");
    assert_eq!(headers["copilot-integration-id"], "vscode-chat");
    assert_eq!(headers["X-Initiator"], "user");
    assert!(headers.get("Copilot-Vision-Request").is_none());
}

#[test]
fn an_image_url_key_without_a_type_marks_the_request_as_vision() {
    let headers = answer("vision_key_without_type");
    assert_eq!(headers["Copilot-Vision-Request"], "true");
}

#[test]
fn a_reasoning_claude_name_gains_thinking_and_reasoning_effort() {
    let params = answer("claude_capability");
    let names: Vec<&str> = params
        .as_array()
        .expect("the answer is a list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&"thinking"));
    assert!(names.contains(&"reasoning_effort"));
}

#[test]
fn a_tool_only_reply_gets_null_content_and_normalized_usage() {
    let reply = answer("tool_only_reply");
    let choice = &reply["choices"][0];
    assert_eq!(choice["finish_reason"], "tool_calls");
    assert_eq!(choice["message"]["content"], Value::Null);
    assert_eq!(
        choice["message"]["tool_calls"][0]["function"]["arguments"],
        r#"{"q": "x"}"#
    );
    assert_eq!(
        reply["usage"],
        json!({"input_tokens": 3_i64, "output_tokens": 4_i64, "prompt_tokens": 3_i64, "completion_tokens": 4_i64, "total_tokens": 7_i64})
    );
}

#[test]
fn an_empty_probe_reply_has_empty_content_and_finish_reason_length() {
    let reply = answer("empty_probe");
    assert_eq!(
        reply["choices"],
        json!([{"index": 0_i64, "message": {"role": "assistant", "content": ""}, "finish_reason": "length"}])
    );
}

#[test]
fn usage_that_already_has_openai_names_is_kept() {
    let reply = answer("usage_already_normalized");
    assert_eq!(
        reply["usage"],
        json!({"prompt_tokens": 2_i64, "completion_tokens": 3_i64, "total_tokens": 9_i64})
    );
    assert_eq!(reply["choices"][0]["finish_reason"], "stop");
}
