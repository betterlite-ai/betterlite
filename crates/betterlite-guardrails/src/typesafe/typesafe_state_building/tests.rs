#![cfg(test)]

use super::build_state;
use crate::typesafe::ELISION_MARKER;
use crate::typesafe::fixtures::{guardrail, json, settings};
use crate::typesafe::typesafe_config::MaxResultChars;

#[test]
fn build_state_uses_the_last_user_message_and_joined_system_messages() {
    let guardrail = guardrail(settings());
    let messages = vec![
        json(r#"{"role":"system","content":"first"}"#),
        json(r#"{"role":"system","content":"second"}"#),
        json(r#"{"role":"user","content":"older task"}"#),
        json(
            r#"{"role":"assistant","tool_calls":[{"id":"a","function":{"name":"lookup","arguments":{"q":1}}}]}"#,
        ),
        json(r#"{"role":"tool","tool_call_id":"a","content":"result"}"#),
        json(r#"{"role":"user","content":"latest task"}"#),
    ];
    let state = build_state(&guardrail, &messages, &[vec![3, 4]]);
    assert_eq!(state.task, "latest task");
    assert_eq!(state.system, "first\n\nsecond");
    assert_eq!(state.tool_exchanges.len(), 1);
    let exchange = state.tool_exchanges.get("e0").expect("e0");
    assert_eq!(exchange.tool_calls.len(), 1);
    assert_eq!(exchange.tool_calls[0].name, Some(json(r#""lookup""#)));
    assert_eq!(exchange.tool_calls[0].arguments, Some(json(r#"{"q":1}"#)));
    assert_eq!(exchange.result, "result");
}

#[test]
fn build_state_defaults_task_and_system_to_empty() {
    let guardrail = guardrail(settings());
    let state = build_state(
        &guardrail,
        &[json(r#"{"role":"assistant","content":"hi"}"#)],
        &[],
    );
    assert_eq!(state.task, "");
    assert_eq!(state.system, "");
    assert!(state.tool_exchanges.is_empty());
}

#[test]
fn build_state_indexes_exchanges_and_truncates_long_results() {
    let mut settings = settings();
    settings.max_result_chars_in_state = MaxResultChars(40);
    let guardrail = guardrail(settings);
    let long = "0123456789".repeat(10);
    let messages = vec![
        json(r#"{"role":"user","content":"task"}"#),
        json(r#"{"role":"assistant","tool_calls":[{"id":"a"}]}"#),
        json(&format!(
            r#"{{"role":"tool","tool_call_id":"a","content":"{long}"}}"#
        )),
        json(r#"{"role":"assistant","tool_calls":[{"id":"b"}]}"#),
        json(&format!(
            r#"{{"role":"tool","tool_call_id":"b","content":"{long}"}}"#
        )),
    ];
    let state = build_state(&guardrail, &messages, &[vec![1, 2], vec![3, 4]]);
    assert_eq!(
        state
            .tool_exchanges
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["e0".to_owned(), "e1".to_owned()]
    );
    assert_eq!(
        state.tool_exchanges["e0"].result,
        format!("012345{ELISION_MARKER}456789")
    );
    assert_eq!(
        state.tool_exchanges["e0"].result,
        state.tool_exchanges["e1"].result
    );
}

#[test]
fn build_state_carries_an_empty_tool_call_list_without_entries() {
    let guardrail = guardrail(settings());
    let messages = vec![
        json(r#"{"role":"assistant","content":"x"}"#),
        json(r#"{"role":"tool","content":"y"}"#),
    ];
    let state = build_state(&guardrail, &messages, &[vec![0, 1]]);
    assert!(state.tool_exchanges["e0"].tool_calls.is_empty());
    assert_eq!(state.tool_exchanges["e0"].result, "y");
}
