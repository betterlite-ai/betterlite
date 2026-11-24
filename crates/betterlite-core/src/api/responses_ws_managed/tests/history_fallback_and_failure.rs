use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const FALLBACK_FRAME: &str =
    r#"{"type": "response.create", "input": "hello", "previous_response_id": "missing"}"#;
const CREATED: &str = r#"{"type": "response.created", "response": {"id": "r-part"}}"#;

#[tokio::test]
async fn an_unknown_previous_response_id_is_handed_to_the_provider_without_a_write() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let provider = support::ScriptedProvider::new(
        &shared,
        vec![support::script(vec![support::event(CREATED)])],
    );

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        FALLBACK_FRAME,
    ))
    .await;
    outcome.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].kwargs.get("stream"),
        Some(&support::json("true"))
    );
    assert_eq!(
        recorded[0].kwargs.get("previous_response_id"),
        Some(&support::json(r#""missing""#))
    );
    assert_eq!(
        recorded[0].kwargs.get("input"),
        Some(&support::json(r#""hello""#))
    );
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: no in-memory history for previous_response_id=missing; falling back to DB-based session reconstruction"
        ]
    );
    assert_eq!(support::history(&handler), support::json("{}"));
}

#[tokio::test]
async fn a_provider_exception_reports_its_message_and_writes_no_history() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let provider =
        support::ScriptedProvider::new(&shared, vec![support::failing_script("upstream boom")]);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let error_frame =
        r#"{"type": "error", "error": {"type": "server_error", "message": "upstream boom"}}"#;
    assert_eq!(support::frames(&shared), vec![support::json(error_frame)]);
    assert_eq!(
        logs,
        vec!["ManagedResponsesWS: error processing response.create: upstream boom"]
    );
    assert_eq!(support::trace(&shared), ["provider"]);
    assert_eq!(support::history(&handler), support::json("{}"));
}

#[tokio::test]
async fn a_stream_exception_reports_its_message_and_writes_no_history() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let chunks = vec![support::event(CREATED)];
    let provider = support::ScriptedProvider::new(
        &shared,
        vec![support::raising_script(chunks, 1, "stream boom")],
    );

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let error_frame =
        r#"{"type": "error", "error": {"type": "server_error", "message": "stream boom"}}"#;
    assert_eq!(
        support::frames(&shared),
        vec![support::json(CREATED), support::json(error_frame)]
    );
    assert_eq!(
        logs,
        vec!["ManagedResponsesWS: error processing response.create: stream boom"]
    );
    assert_eq!(support::history(&handler), support::json("{}"));
}
