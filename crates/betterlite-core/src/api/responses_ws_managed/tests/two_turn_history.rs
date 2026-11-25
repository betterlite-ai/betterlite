use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const FIRST: &str = r#"{"type": "response.completed", "response": {"id": "r1", "output": [{"type": "message", "content": [{"type": "output_text", "text": "A"}]}]}}"#;
const SECOND: &str = r#"{"type": "response.completed", "response": {"id": "r2", "output": [{"type": "message", "content": [{"type": "output_text", "text": "B"}]}]}}"#;
const FIRST_FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const SECOND_FRAME: &str =
    r#"{"type": "response.create", "input": "again", "previous_response_id": "r1"}"#;
const SECOND_INPUT: &str = r#"[{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]}, {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]}, {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "again"}]}]"#;
const STORED_FIRST: &str = r#"{"r1": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]}
]}"#;
const STORED_SECOND: &str = r#"{"r1": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]}
], "r2": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]},
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "again"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "B"}]}
]}"#;

#[tokio::test]
async fn a_completed_turn_is_stored_under_its_response_id() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let provider =
        support::ScriptedProvider::new(&shared, vec![support::script(vec![support::event(FIRST)])]);

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        FIRST_FRAME,
    ))
    .await;
    outcome.unwrap();
    assert_eq!(
        logs,
        vec!["ManagedResponsesWS: stored 2 messages for response_id=r1"]
    );
    assert_eq!(support::history(&handler), support::json(STORED_FIRST));
}

#[tokio::test]
async fn a_known_previous_response_id_reuses_the_stored_messages() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let scripts = vec![
        support::script(vec![support::event(FIRST)]),
        support::script(vec![support::event(SECOND)]),
    ];
    let provider = support::ScriptedProvider::new(&shared, scripts);

    let (first, _first_logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        FIRST_FRAME,
    ))
    .await;
    first.unwrap();
    let (second, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        SECOND_FRAME,
    ))
    .await;
    second.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 2);
    let call = &recorded[1];
    assert_eq!(call.kwargs.get("input"), Some(&support::json(SECOND_INPUT)));
    assert!(!call.kwargs.contains_key("previous_response_id"));
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: prepended 2 history messages for previous_response_id=r1",
            "ManagedResponsesWS: stored 4 messages for response_id=r2",
        ]
    );
    assert_eq!(support::history(&handler), support::json(STORED_SECOND));
}
