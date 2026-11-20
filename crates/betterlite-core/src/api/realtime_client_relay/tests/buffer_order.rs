use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::api::realtime_contracts::close::ClientLoopExit;

use super::harness::*;

const AUDIO_APPEND: &str = r#"{"type":"input_audio_buffer.append","audio":"AAAA"}"#;
const TEXT_FRAME: &str = r#"{"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#;

#[tokio::test]
async fn client_buffer_and_logging_order() {
    pre_setup_audio_is_buffered().await;
    the_setup_update_goes_before_the_backlog().await;
    unrelated_frames_flush_once_setup_is_complete().await;
    arrivals_during_flushing_are_buffered().await;
    a_failed_flush_retains_the_queue().await;
    rewritten_payloads_are_logged().await;
    receiver_failure_logs_and_disconnects().await;
    backend_failure_returns_backend_closed().await;
    a_client_notice_failure_takes_the_generic_path().await;
}

async fn pre_setup_audio_is_buffered() {
    let outcome = ack(Fixture::default()
        .with_provider(false, true)
        .with_client_message(AUDIO_APPEND))
    .await;
    assert!(outcome.backend.frames().is_empty());
    assert_eq!(
        outcome.state.setup.pending_messages_until_setup,
        vec![String::from(AUDIO_APPEND)]
    );
    assert_eq!(outcome.pre_calls(), vec![String::from(AUDIO_APPEND)]);
}

async fn the_setup_update_goes_before_the_backlog() {
    let update = r#"{"type":"session.update","session":{"instructions":"hi"}}"#;
    let outcome = ack(Fixture::default()
        .with_provider(false, true)
        .with_pending_messages(&[AUDIO_APPEND])
        .with_client_message(update))
    .await;
    assert_eq!(
        outcome.backend.parsed(),
        vec![json(
            r#"{"type":"session.update","session":{"type":"realtime","instructions":"hi"}}"#
        )]
    );
    assert_eq!(
        outcome.state.setup.pending_messages_until_setup,
        vec![String::from(AUDIO_APPEND)]
    );
}

async fn unrelated_frames_flush_once_setup_is_complete() {
    let outcome = ack(Fixture::default()
        .with_provider(false, true)
        .with_setup_complete(true)
        .with_pending_messages(&[AUDIO_APPEND])
        .with_client_message(TEXT_FRAME))
    .await;
    assert_eq!(
        outcome.backend.frames(),
        vec![String::from(AUDIO_APPEND), String::from(TEXT_FRAME)]
    );
    assert!(outcome.state.setup.pending_messages_until_setup.is_empty());
}

async fn arrivals_during_flushing_are_buffered() {
    let outcome = ack(Fixture::default()
        .with_provider(false, true)
        .with_setup_complete(true)
        .with_flushing(true)
        .with_client_message(AUDIO_APPEND)
        .with_client_message(TEXT_FRAME))
    .await;
    assert!(outcome.backend.frames().is_empty());
    assert_eq!(
        outcome.state.setup.pending_messages_until_setup,
        vec![String::from(AUDIO_APPEND), String::from(TEXT_FRAME)]
    );
}

async fn a_failed_flush_retains_the_queue() {
    let outcome = ack(Fixture::default()
        .with_provider(false, true)
        .with_setup_complete(true)
        .with_pending_messages(&[AUDIO_APPEND])
        .with_backend_failures(vec![0])
        .with_client_message(TEXT_FRAME))
    .await;
    assert_eq!(outcome.backend.attempts(), 1);
    assert_eq!(
        outcome.state.setup.pending_messages_until_setup,
        vec![String::from(AUDIO_APPEND), String::from(TEXT_FRAME)]
    );
}

async fn rewritten_payloads_are_logged() {
    let update = r#"{"type":"session.update","session":{"instructions":"hi"}}"#;
    let outcome = ack(Fixture::default()
        .with_normalizer()
        .with_client_message(update))
    .await;
    assert_eq!(outcome.pre_calls(), outcome.backend.frames());
    assert_eq!(
        outcome.parsed_pre_calls(),
        vec![json(
            r#"{"type":"session.update","session":{"type":"realtime","instructions":"hi","xai_patch":true}}"#
        )]
    );
}

async fn receiver_failure_logs_and_disconnects() {
    let (logs, _guard) = capture();
    let outcome = ack(Fixture::default().with_client_failure("gone")).await;
    assert_eq!(outcome.exit, ClientLoopExit::ClientDisconnected);
    assert!(logs.starts_with("Client disconnected: "));
}

async fn backend_failure_returns_backend_closed() {
    let (logs, _guard) = capture();
    let outcome = ack(Fixture::default()
        .with_backend_failures(vec![0])
        .with_client_message(r#"{"type":"response.create"}"#))
    .await;
    assert_eq!(outcome.exit, ClientLoopExit::BackendClosed);
    assert!(logs.starts_with("Backend closed while forwarding a client message: "));
}

async fn a_client_notice_failure_takes_the_generic_path() {
    let (logs, _guard) = capture();
    let guardrail = ScriptedGuardrail::blocking(vec![GuardrailEventHooks::PreCall], "block me");
    let outcome = ack(
        Fixture::default()
            .with_guardrail(guardrail)
            .with_client_send_failures(vec![0])
            .with_client_message(
                r#"{"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":"block me"}]}}"#,
            ),
    )
    .await;
    assert_eq!(outcome.exit, ClientLoopExit::ClientDisconnected);
    assert!(logs.starts_with("Error in client ack messages: "));
}
