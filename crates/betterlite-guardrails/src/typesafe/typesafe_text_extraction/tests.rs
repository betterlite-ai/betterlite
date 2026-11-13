#![cfg(test)]

use super::exchange_tool_text;
use crate::typesafe::fixtures::{guardrail, json, settings};

#[test]
fn exchange_tool_text_concatenates_tool_and_function_content() {
    let guardrail = guardrail(settings());
    let messages = vec![
        json(r#"{"role":"assistant","tool_calls":[{"id":"a"}]}"#),
        json(r#"{"role":"tool","tool_call_id":"a","content":"one"}"#),
        json(r#"{"role":"function","name":"f","content":"two"}"#),
    ];
    assert_eq!(
        exchange_tool_text(&guardrail, &messages, &[0, 1, 2]),
        "onetwo"
    );
}

#[test]
fn exchange_tool_text_skips_other_roles_and_missing_indices() {
    let guardrail = guardrail(settings());
    let messages = vec![
        json(r#"{"role":"assistant","content":"zero"}"#),
        json(r#"{"role":"user","content":"one"}"#),
        json(r#"{"role":"tool","content":"two"}"#),
    ];
    assert_eq!(
        exchange_tool_text(&guardrail, &messages, &[0, 1, 2, 9]),
        "two"
    );
}

#[test]
fn exchange_tool_text_uses_content_to_text_on_part_lists() {
    let guardrail = guardrail(settings());
    let messages = vec![
        json(r#"{"role":"assistant"}"#),
        json(
            r#"{"role":"tool","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{}},{"type":"text","text":"b"}]}"#,
        ),
    ];
    assert_eq!(exchange_tool_text(&guardrail, &messages, &[0, 1]), "a\n\nb");
}

#[test]
fn exchange_tool_text_treats_missing_content_as_empty() {
    let guardrail = guardrail(settings());
    let messages = vec![json(r#"{"role":"assistant"}"#), json(r#"{"role":"tool"}"#)];
    assert_eq!(exchange_tool_text(&guardrail, &messages, &[0, 1]), "");
}
