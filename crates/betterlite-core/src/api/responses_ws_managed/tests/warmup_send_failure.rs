use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const SESSION_FRAME: &str = r#"{"type": "response.create", "generate": false}"#;

#[tokio::test]
async fn a_failed_warmup_send_is_swallowed_without_an_error_frame() {
    let shared = support::shared();
    let sink = support::failing_sink(&shared, 1);
    let mut handler = support::handler(support::connection(support::MODEL, sink));
    let provider = support::ScriptedProvider::idle(&shared);

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        SESSION_FRAME,
    ))
    .await;
    outcome.unwrap();
    let diagnostic = format!(
        "ManagedResponsesWS: error sending warmup ack: {}",
        support::SEND_FAILED
    );
    assert_eq!(logs, vec![diagnostic]);
    assert!(support::frames(&shared).is_empty());
    assert!(support::calls(&shared).is_empty());
    assert!(support::quotas(&shared).is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
}
