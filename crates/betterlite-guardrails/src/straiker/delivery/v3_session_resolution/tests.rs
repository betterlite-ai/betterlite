#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::straiker::delivery::StraikerDelivery;
use crate::straiker::delivery::fixtures::{
    FakeDelay, FakePost, Fakes, Sent, V3_ALLOW, delivery, inputs, object, reply, request_data, run,
    sent, settings, v3_raw,
};
use crate::straiker::v3::SESSION_HEADER;

fn posted_at(posts: &[Sent], index: usize) -> Object {
    object(&posts[index].body)
}

fn session_of(fields: &Object) -> Option<String> {
    match fields.get("session_id") {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn session_at(handler: &StraikerDelivery<FakePost, FakeDelay>, index: usize) -> Option<String> {
    session_of(&posted_at(&sent(handler), index))
}

fn header_at(handler: &StraikerDelivery<FakePost, FakeDelay>, index: usize) -> Option<String> {
    sent(handler)[index].headers.get(SESSION_HEADER).cloned()
}

#[tokio::test]
async fn the_wire_session_header_wins_and_is_trimmed() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","litellm_session_id":"resolved","proxy_server_request":{"headers":{"x-claude-code-session-id":" session-1 "}},"messages":[{"role":"user","content":"hello"}]}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0).as_deref(), Some("session-1"));
    assert_eq!(header_at(&handler, 0).as_deref(), Some("session-1"));
}

#[tokio::test]
async fn an_empty_wire_session_falls_through_to_the_resolved_one() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","litellm_session_id":"resolved","proxy_server_request":{"headers":{"x-claude-code-session-id":""}},"messages":[{"role":"user","content":"hello"}]}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0).as_deref(), Some("resolved"));
    assert_eq!(header_at(&handler, 0), None);
}

#[tokio::test]
async fn a_resolved_session_is_read_from_the_litellm_field() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","litellm_session_id":"resolved","messages":[{"role":"user","content":"hello"}]}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0).as_deref(), Some("resolved"));
}

#[tokio::test]
async fn a_resolved_session_can_be_a_number() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","litellm_session_id":42,"messages":[{"role":"user","content":"hello"}]}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0).as_deref(), Some("42"));
}

#[tokio::test]
async fn the_metadata_session_beats_the_litellm_metadata_one() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","metadata":{"session_id":"metadata-session"},"litellm_metadata":{"session_id":"trace-session"},"messages":[{"role":"user","content":"hello"}]}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0).as_deref(), Some("metadata-session"));
}

#[tokio::test]
async fn the_litellm_metadata_session_is_used_last() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","litellm_metadata":{"session_id":"trace-session"},"messages":[{"role":"user","content":"hello"}]}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0).as_deref(), Some("trace-session"));
}

#[tokio::test]
async fn a_session_is_derived_steadily_from_the_first_message() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_ALLOW), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#;
    let mut first = request_data(body);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let mut second = request_data(body);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let derived = session_at(&handler, 0).expect("a conversation session");
    assert!(derived.starts_with("litellm-"));
    assert_eq!(derived.len(), "litellm-".len() + 32);
    let hex = &derived["litellm-".len()..];
    assert!(
        hex.chars()
            .all(|letter| letter.is_ascii_hexdigit() && !letter.is_ascii_uppercase())
    );
    assert_eq!(session_at(&handler, 1), Some(derived));
}

#[tokio::test]
async fn the_derived_session_separates_principals() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_ALLOW), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let messages = r#"[{"role":"user","content":"hello"}]"#;
    let mut alice = request_data(&format!(
        r#"{{"model":"gpt-4o","metadata":{{"user_api_key_user_id":"alice"}},"messages":{messages}}}"#
    ));
    run(
        &handler,
        &mut alice,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let mut bob = request_data(&format!(
        r#"{{"model":"gpt-4o","metadata":{{"user_api_key_user_id":"bob"}},"messages":{messages}}}"#
    ));
    run(
        &handler,
        &mut bob,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let alice_session = session_at(&handler, 0).expect("alice's session");
    let bob_session = session_at(&handler, 1).expect("bob's session");
    assert_ne!(alice_session, bob_session);
}

#[tokio::test]
async fn the_derived_session_carries_the_system_prompt() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_ALLOW), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(
        r#"{"model":"gpt-4o","system":"be nice","messages":[{"role":"user","content":"hello"}]}"#,
    );
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let mut second = request_data(
        r#"{"model":"gpt-4o","system":"be terse","messages":[{"role":"user","content":"hello"}]}"#,
    );
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_ne!(session_at(&handler, 0), session_at(&handler, 1));
}

#[tokio::test]
async fn a_call_with_no_conversation_has_no_session() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(session_at(&handler, 0), None);
}

#[tokio::test]
async fn the_response_phase_carries_the_derived_session() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data = request_data(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"response":{"id":"r1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}]}}"#,
    );
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect("an allowed call");
    let derived = session_at(&handler, 0).expect("the response joins the conversation");
    assert!(derived.starts_with("litellm-"));
}
