use super::support;
use crate::api::responses_ws_managed::process::process_response_create;
use rstest::rstest;

const SESSION_FRAME: &str = r#"{"type": "response.create", "generate": false}"#;

#[rstest]
#[case::first_ack(1, 0)]
#[case::second_ack(2, 1)]
#[tokio::test]
async fn a_failed_warmup_send_is_swallowed_without_an_error_frame(
    #[case] fails_at: usize,
    #[case] sent: usize,
) {
    let shared = support::shared();
    let sink = support::failing_sink(&shared, fails_at);
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
    assert_eq!(support::frames(&shared).len(), sent);
    assert!(support::calls(&shared).is_empty());
    assert!(support::quotas(&shared).is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
}
