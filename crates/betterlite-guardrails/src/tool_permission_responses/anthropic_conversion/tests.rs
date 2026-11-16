#![cfg(test)]

use super::{anthropic_content_blocks, anthropic_to_tool_call};
use crate::tool_permission_responses::tests::support::{json, text};

#[test]
fn anthropic_to_tool_call_converts_a_tool_use_block() {
    let block = json(r#"{"type":"tool_use","id":"toolu_1","name":"read","input":{"path":"a"}}"#);
    let call = anthropic_to_tool_call(&block).expect("tool use converts");
    assert_eq!(call.id, "toolu_1");
    assert_eq!(call.kind.as_deref(), Some("function"));
    assert_eq!(call.name.as_deref(), Some("read"));
    assert_eq!(call.arguments, Some(json(r#"{"path":"a"}"#)));
}

#[test]
fn anthropic_to_tool_call_requires_a_non_empty_name() {
    assert!(anthropic_to_tool_call(&json(r#"{"type":"tool_use","id":"toolu_1"}"#)).is_none());
    assert!(anthropic_to_tool_call(&json(r#"{"type":"tool_use","name":"","id":"x"}"#)).is_none());
    assert!(anthropic_to_tool_call(&json(r#"{"type":"tool_use","name":7}"#)).is_none());
}

#[test]
fn anthropic_to_tool_call_defaults_a_missing_or_non_text_id_to_empty() {
    let missing =
        anthropic_to_tool_call(&json(r#"{"type":"tool_use","name":"read"}"#)).expect("call");
    assert_eq!(missing.id, "");
    let non_text =
        anthropic_to_tool_call(&json(r#"{"type":"tool_use","name":"read","id":9}"#)).expect("call");
    assert_eq!(non_text.id, "");
}

#[test]
fn anthropic_to_tool_call_uses_braces_for_a_non_object_input() {
    let missing =
        anthropic_to_tool_call(&json(r#"{"type":"tool_use","name":"read"}"#)).expect("call");
    assert_eq!(missing.arguments, Some(text("{}")));
    let list = anthropic_to_tool_call(&json(r#"{"type":"tool_use","name":"read","input":[1]}"#))
        .expect("call");
    assert_eq!(list.arguments, Some(text("{}")));
}

#[test]
fn anthropic_to_tool_call_ignores_other_block_types() {
    assert!(anthropic_to_tool_call(&json(r#"{"type":"text","text":"hi"}"#)).is_none());
    assert!(anthropic_to_tool_call(&json(r#"{"text":"hi"}"#)).is_none());
    assert!(anthropic_to_tool_call(&text("hi")).is_none());
}

#[test]
fn anthropic_content_blocks_needs_an_object_with_a_content_list() {
    let response = json(r#"{"content":[{"type":"text","text":"hi"}]}"#);
    assert_eq!(anthropic_content_blocks(&response).map(<[_]>::len), Some(1));
    assert!(anthropic_content_blocks(&text("hi")).is_none());
    assert!(anthropic_content_blocks(&json(r#"{"content":"hi"}"#)).is_none());
    assert!(anthropic_content_blocks(&json(r#"{}"#)).is_none());
    assert!(anthropic_content_blocks(&json(r#"{"content":null}"#)).is_none());
}
