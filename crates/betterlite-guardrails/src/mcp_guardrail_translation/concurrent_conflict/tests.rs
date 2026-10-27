#![cfg(test)]

use crate::mcp_guardrail_translation::error::ContentBlocked;
use crate::mcp_guardrail_translation::input_writeback::write_back;
use crate::mcp_guardrail_translation::nesting_depth::{LeafPath, PathPart};
use crate::mcp_guardrail_translation::process_input::InputScan;
use crate::mcp_guardrail_translation::tests::{request, texts, to_json};

const SNAPSHOT: &str =
    r#"{"mcp_tool_name":"send","mcp_arguments":{"to":"bob@example.com","body":"hi"}}"#;

fn path(keys: &[&str]) -> LeafPath {
    LeafPath(
        keys.iter()
            .map(|key| PathPart::Key((*key).to_owned()))
            .collect(),
    )
}

#[test]
fn two_different_rewrites_of_one_leaf_are_rejected() {
    let mut call = request(SNAPSHOT);
    let first = InputScan::of(&call).unwrap().unwrap();
    let second = InputScan::of(&call).unwrap().unwrap();
    assert_eq!(first.inputs.texts, texts(&["hi", "bob@example.com"]));
    write_back(&first, &mut call, Some(texts(&["hi", "[EMAIL]"]))).unwrap();
    let outcome = write_back(
        &second,
        &mut call,
        Some(texts(&["hi", "[PERSON]@example.com"])),
    );
    assert_eq!(
        outcome,
        Err(ContentBlocked::ConflictingRewrite {
            paths: vec![path(&["to"])]
        })
    );
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "content blocked: two guardrails running in parallel rewrote the same MCP tool call argument (to) differently; remove run_in_parallel from one of them"
    );
    assert_eq!(
        to_json(&call.mcp_arguments),
        r#"{"body":"hi","to":"[EMAIL]"}"#
    );
}

#[test]
fn the_same_rewrite_twice_is_safe() {
    let mut call = request(SNAPSHOT);
    let first = InputScan::of(&call).unwrap().unwrap();
    let second = InputScan::of(&call).unwrap().unwrap();
    write_back(&first, &mut call, Some(texts(&["hi", "[EMAIL]"]))).unwrap();
    write_back(&second, &mut call, Some(texts(&["hi", "[EMAIL]"]))).unwrap();
    assert_eq!(
        to_json(&call.mcp_arguments),
        r#"{"body":"hi","to":"[EMAIL]"}"#
    );
}

#[test]
fn rewrites_of_different_leaves_merge() {
    let mut call = request(SNAPSHOT);
    let first = InputScan::of(&call).unwrap().unwrap();
    let second = InputScan::of(&call).unwrap().unwrap();
    write_back(&first, &mut call, Some(texts(&["hi", "[EMAIL]"]))).unwrap();
    write_back(
        &second,
        &mut call,
        Some(texts(&["[GREETING]", "bob@example.com"])),
    )
    .unwrap();
    assert_eq!(
        to_json(&call.mcp_arguments),
        r#"{"body":"[GREETING]","to":"[EMAIL]"}"#
    );
}

#[test]
fn reshaped_arguments_reject_every_rewrite() {
    let mut call = request(SNAPSHOT);
    let scan = InputScan::of(&call).unwrap().unwrap();
    call.mcp_arguments =
        request(r#"{"mcp_arguments":{"to":["bob@example.com"],"body":"hi"}}"#).mcp_arguments;
    let outcome = write_back(&scan, &mut call, Some(texts(&["[GREETING]", "[EMAIL]"])));
    assert_eq!(
        outcome,
        Err(ContentBlocked::ConflictingRewrite {
            paths: vec![path(&["body"]), path(&["to"])]
        })
    );
}
