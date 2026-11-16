#![cfg(test)]

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::straiker::delivery::fixtures::{
    Fakes, V3_ALLOW, V3_BLOCK, capture_fields, delivery, inputs, reply, request_data, run, sent,
    settings, v3_raw,
};
use crate::third_party_init_b::straiker::StraikerArgs;

const HELLO: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#;
const GROWN: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"},{"role":"assistant","content":"hi"},{"role":"user","content":"again"}]}"#;

fn scoped(session: &str, messages: &str) -> String {
    format!(
        r#"{{"model":"gpt-4o","proxy_server_request":{{"headers":{{"x-claude-code-session-id":"{session}"}}}},"messages":{messages}}}"#
    )
}

fn owned(session: &str, owner: &str, messages: &str) -> String {
    format!(
        r#"{{"model":"gpt-4o","proxy_server_request":{{"headers":{{"x-claude-code-session-id":"{session}"}}}},"metadata":{{"user_api_key_user_id":"{owner}"}},"messages":{messages}}}"#
    )
}

fn blocked(failure: GuardrailFailure) -> (String, bool) {
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        ..
    } = failure
    else {
        panic!("expected a raised block");
    };
    (message, blocked_content)
}

fn verbose_v3() -> StraikerArgs {
    let mut args = v3_raw();
    args.verbose = Some(true);
    args
}

#[tokio::test]
async fn a_resend_of_a_blocked_turn_is_blocked_without_a_post() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_BLOCK)]);
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), ("policy".to_owned(), true));
    assert_eq!(sent(&handler).len(), 1);
    let mut second = request_data(HELLO);
    let failure = run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the replay is blocked here");
    assert_eq!(blocked(failure), ("policy".to_owned(), true));
    assert_eq!(sent(&handler).len(), 1);
}

#[tokio::test]
async fn a_conversation_grown_past_a_blocked_turn_is_blocked() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_BLOCK)]);
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut grown = request_data(GROWN);
    let failure = run(
        &handler,
        &mut grown,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the blocked exchange is part of the history");
    assert_eq!(blocked(failure), ("policy".to_owned(), true));
    assert_eq!(sent(&handler).len(), 1);
}

#[tokio::test]
async fn a_different_conversation_in_the_same_session_is_asked() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(&scoped(
        "session-1",
        r#"[{"role":"user","content":"hello"}]"#,
    ));
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut other = request_data(&scoped(
        "session-1",
        r#"[{"role":"user","content":"good morning"}]"#,
    ));
    run(
        &handler,
        &mut other,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("another conversation is judged on its own");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_different_session_is_asked() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let messages = r#"[{"role":"user","content":"hello"}]"#;
    let mut first = request_data(&scoped("session-1", messages));
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut other = request_data(&scoped("session-2", messages));
    run(
        &handler,
        &mut other,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("another session is judged on its own");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_different_principal_is_asked() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let messages = r#"[{"role":"user","content":"hello"}]"#;
    let mut first = request_data(&owned("session-1", "alice", messages));
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut other = request_data(&owned("session-1", "bob", messages));
    run(
        &handler,
        &mut other,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("another person is judged on their own");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_block_without_a_fingerprinted_turn_is_not_remembered() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let body = r#"{"model":"gpt-4o","metadata":{"user_api_key_user_id":"alice"}}"#;
    let mut first = request_data(body);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), ("policy".to_owned(), true));
    let mut second = request_data(body);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("there is no turn to remember");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_response_phase_block_is_not_remembered() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
    );
    let call = fakes.call(Some("completion"), None);
    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"response":{"id":"r1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}]}}"#;
    let mut first = request_data(body);
    let failure = run(
        &handler,
        &mut first,
        inputs(&["answer"]),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect_err("the answer blocks");
    assert_eq!(blocked(failure), ("policy".to_owned(), true));
    let mut second = request_data(body);
    run(
        &handler,
        &mut second,
        inputs(&["answer"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the request phase was never blocked");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn the_replay_is_logged_when_verbose() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&verbose_v3()), vec![reply(200, V3_BLOCK)]);
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut second = request_data(HELLO);
    let (fields, _guard) = capture_fields();
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the replay is blocked here");
    let fields = fields.take();
    assert_eq!(message_list(&fields), vec!["straiker.replay_blocked"]);
    let prefix = value_of(&fields, "prefix");
    assert_eq!(prefix.len(), 64);
    assert!(value_of(&fields, "scope").contains("litellm-"));
}

#[tokio::test]
async fn the_replay_is_silent_without_verbose() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_BLOCK)]);
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut second = request_data(HELLO);
    let (fields, _guard) = capture_fields();
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the replay is blocked here");
    assert!(message_list(&fields.take()).is_empty());
}

fn value_of(fields: &[(String, String)], name: &str) -> String {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn message_list(fields: &[(String, String)]) -> Vec<String> {
    fields
        .iter()
        .filter(|(key, _)| key == "message")
        .map(|(_, value)| value.clone())
        .collect()
}
