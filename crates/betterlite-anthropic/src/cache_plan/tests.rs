#![cfg(test)]

use rstest::rstest;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

use crate::cache_plan::{UnsupportedCacheReason, parse_cache_plan, parse_prompt};

#[test]
fn cache_plan_builds_automatic_breakpoint_and_prefix() {
    let body = json!({
        "model": "claude",
        "max_tokens": 20,
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}],
        "cache_control": {"type": "ephemeral", "ttl": "1h"}
    });
    let plan = parse_cache_plan(body.as_object().expect("object")).expect("supported");
    assert_eq!(plan.breakpoints.len(), 1);
    assert_eq!(plan.breakpoints[0].ttl_seconds, 3600);
    assert_eq!(
        plan.breakpoints[0].prefix_body["messages"][0]["content"][0]["text"],
        "hello"
    );
}

#[test]
fn cache_plan_rejects_unknown_fields_and_reversed_ttl_order() {
    let unknown = json!({"messages": [{"role": "user", "content": "hello"}], "unknown": true});
    assert_eq!(
        parse_cache_plan(unknown.as_object().expect("object"))
            .expect_err("invalid")
            .reason,
        UnsupportedCacheReason::UnsupportedPromptShape
    );
    let reversed = json!({
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "5m"}},
                {"type": "text", "text": "b", "cache_control": {"type": "ephemeral", "ttl": "1h"}}
            ]
        }]
    });
    assert_eq!(
        parse_cache_plan(reversed.as_object().expect("object"))
            .expect_err("invalid")
            .reason,
        UnsupportedCacheReason::InvalidCacheTtlOrder
    );
}

#[test]
fn prompt_view_rejects_thinking_and_multiple_breakpoints() {
    let thinking = json!({"messages": [{"role": "assistant", "content": [{"type": "thinking", "thinking": "x", "signature": "s"}]}]});
    assert!(parse_prompt(thinking.as_object().expect("object")).is_none());
    let multiple = json!({"messages": [{"role": "user", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}, {"type": "text", "text": "b", "cache_control": {"type": "ephemeral"}}]}]});
    assert!(parse_prompt(multiple.as_object().expect("object")).is_none());
}

#[rstest]
fn replays_the_cache_plan_corpus(#[files("fuzz/corpus/cache_plan_decoder/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("corpus JSON");
    let body = value.as_object().expect("corpus object");
    drop(parse_cache_plan(body));
}
