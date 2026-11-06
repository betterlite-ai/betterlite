#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::build_full_history;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::xecguard_primitives::wire::XecGuardMessage;

fn request(wire: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(wire).expect("request data")
}

fn message(role: &str, content: &str) -> XecGuardMessage {
    XecGuardMessage {
        role: role.to_owned(),
        content: content.to_owned(),
    }
}

#[test]
fn history_keeps_the_system_prompt_and_the_whole_conversation() {
    let data = request(
        r#"{"messages":[{"role":"system","content":"policy"},{"role":"user","content":"first"},{"role":"assistant","content":"reply"},{"role":"user","content":"latest"}]}"#,
    );
    let history = build_full_history(&data, &["ignored".to_owned()], GuardrailInputType::Request);
    assert_eq!(
        history,
        vec![
            message("system", "policy"),
            message("user", "first"),
            message("assistant", "reply"),
            message("user", "latest"),
        ]
    );
}

#[test]
fn history_ignores_framework_texts_when_messages_exist() {
    let data = request(
        r#"{"messages":[{"role":"user","content":"from history"}],"structured_messages":[]}"#,
    );
    let history = build_full_history(
        &data,
        &["from texts".to_owned()],
        GuardrailInputType::Request,
    );
    assert_eq!(history, vec![message("user", "from history")]);
}

#[test]
fn history_synthesizes_a_user_turn_from_the_input_texts() {
    let data = request(r#"{"messages":[{"role":"system","content":"policy"}]}"#);
    let history = build_full_history(
        &data,
        &["one".to_owned(), String::new(), "two".to_owned()],
        GuardrailInputType::Request,
    );
    assert_eq!(
        history,
        vec![message("system", "policy"), message("user", "one\ntwo")]
    );
    assert!(build_full_history(&data, &[], GuardrailInputType::Request).is_empty());
}

#[test]
fn history_is_empty_without_messages_or_synthesizable_texts() {
    let data = request(r#"{"messages":[]}"#);
    assert!(
        build_full_history(&data, &["text".to_owned()], GuardrailInputType::Request).is_empty()
    );
    let non_object = request(r#"{"messages":["x",3,null,{"role":"assistant","content":"a"}]}"#);
    assert_eq!(
        build_full_history(
            &non_object,
            &["text".to_owned()],
            GuardrailInputType::Request
        ),
        vec![message("assistant", "a"), message("user", "text")]
    );
}

#[test]
fn history_appends_the_assistant_text_for_response_scans() {
    let data = request(
        r#"{"messages":[{"role":"system","content":"policy"},{"role":"user","content":"question"}],"response":{"choices":[{"message":{"content":"answer"}}]}}"#,
    );
    let history = build_full_history(&data, &[], GuardrailInputType::Response);
    assert_eq!(
        history,
        vec![
            message("system", "policy"),
            message("user", "question"),
            message("assistant", "answer"),
        ]
    );
}

#[test]
fn history_is_empty_for_a_response_without_extractable_text() {
    let data = request(r#"{"messages":[{"role":"user","content":"question"}]}"#);
    assert!(build_full_history(&data, &[], GuardrailInputType::Response).is_empty());
    let empty_response =
        request(r#"{"messages":[{"role":"user","content":"question"}],"response":null}"#);
    assert!(build_full_history(&empty_response, &[], GuardrailInputType::Response).is_empty());
}
