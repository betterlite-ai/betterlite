use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const CREATED: &str = r#"{"type": "response.created", "response": {"id": "r-send"}}"#;
const COMPLETED: &str = r#"{"type": "response.completed", "response": {"id": "r-send", "output": [{"type": "message", "content": [{"type": "output_text", "text": "S"}]}]}}"#;
const LATE: &str = r#"{"type": "response.output_text.delta", "delta": "late"}"#;
const FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const STORED: &str = r#"{"r-send": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "S"}]}
]}"#;

#[tokio::test]
async fn a_client_send_failure_stops_forwarding_but_keeps_the_captured_completion() {
    let shared = support::shared();
    let sink = support::failing_sink(&shared, 2);
    let mut handler = support::handler(support::connection(support::MODEL, sink));
    let chunks = vec![
        support::event(CREATED),
        support::event(COMPLETED),
        support::event(LATE),
    ];
    let provider = support::ScriptedProvider::new(&shared, vec![support::script(chunks)]);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    assert_eq!(support::frames(&shared), vec![support::json(CREATED)]);
    let diagnostic = format!(
        "ManagedResponsesWS: error sending chunk to client: {}",
        support::SEND_FAILED
    );
    let stored = "ManagedResponsesWS: stored 2 messages for response_id=r-send".to_owned();
    assert_eq!(logs, vec![diagnostic, stored]);
    assert_eq!(support::history(&handler), support::json(STORED));
    assert_eq!(support::trace(&shared), ["provider"]);
}
