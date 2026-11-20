use betterlite_models::decisions::Json;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::error::RealtimeRelayError;
use crate::api::realtime_backend_relay::relay::FAILURE_LOGGED_KEY;
use crate::api::realtime_session_state::logging::SUCCESS_LOGGED_KEY;

use super::harness::*;

#[tokio::test]
async fn backend_close_and_logging() {
    let (logs, _guard) = capture();
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.close_with(1008, "policy");
    let options = StateOptions {
        logging: Logging::LoggerAndWorker,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    let close = result.unwrap();
    assert_eq!(close.code, 1008);
    assert_eq!(close.reason, "policy");
    let refusals = fixture.dispatch.refusals();
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].code, 1008);
    assert_eq!(refusals[0].message, "policy");
    assert_eq!(fixture.dispatch.prefer_async(), vec![true]);
    assert_eq!(
        fixture.logging.detail(FAILURE_LOGGED_KEY),
        Some(Json::Bool(true))
    );
    assert_eq!(fixture.logging.dispatches(), 0);
    assert_eq!(fixture.queue.enqueues(), 1);
    assert!(logs.starts_with("Connection closed in backend to client send messages - "));
}

#[tokio::test]
async fn close_1000_before_frames_logs_success() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.close_with(1000, "");
    let options = StateOptions {
        logging: Logging::LoggerAndWorker,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(fixture.logging.dispatches(), 1);
    assert_eq!(
        fixture.logging.detail(SUCCESS_LOGGED_KEY),
        Some(Json::Bool(true))
    );
    assert!(fixture.dispatch.refusals().is_empty());
}

#[tokio::test]
async fn close_1008_after_a_frame_logs_success() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text("not json");
    source.close_with(1008, "policy");
    let options = StateOptions {
        logging: Logging::LoggerAndWorker,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1008);
    assert_eq!(fixture.logging.dispatches(), 1);
    assert!(fixture.dispatch.refusals().is_empty());
}

#[tokio::test]
async fn close_without_a_received_frame_maps_to_1006() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.close(None);
    let options = StateOptions {
        logging: Logging::LoggerAndWorker,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    let close = result.unwrap();
    assert_eq!(close.code, 1006);
    assert_eq!(close.reason, "the scripted backend closed without a frame");
    let refusals = fixture.dispatch.refusals();
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].code, 1006);
}

#[tokio::test]
async fn missing_logger_makes_failure_logging_a_no_op() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.close_with(1008, "policy");
    let token = CancellationToken::new();
    let (result, _state) = run(&StateOptions::default(), &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1008);
    assert!(fixture.dispatch.refusals().is_empty());
    assert!(fixture.logging.details().is_empty());
    assert_eq!(fixture.queue.enqueues(), 0);
}

#[tokio::test]
async fn cancellation_flushes_usage_logs_success_and_propagates() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    fixture.provider.unbilled_seconds = Some(1);
    let mut source = fixture.source.clone();
    let options = StateOptions {
        provider: Flag(true),
        logging: Logging::LoggerAndWorker,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    token.cancel();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert!(matches!(result, Err(RealtimeRelayError::Cancelled)));
    assert!(source.decode_calls().is_empty());
    assert_eq!(fixture.logging.dispatches(), 1);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from(COMPLETED)]
    );
    assert!(state.messages[0].get("usage").is_some());
}

#[tokio::test]
async fn raw_client_failure_ends_the_relay_with_1011() {
    let (logs, _guard) = capture();
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text("not json");
    let options = StateOptions {
        logging: Logging::LoggerAndWorker,
        client_failures: vec![0],
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    let close = result.unwrap();
    assert_eq!(close.code, 1011);
    assert_eq!(
        close.reason,
        "proxy failed while relaying the upstream websocket"
    );
    assert_eq!(fixture.logging.dispatches(), 1);
    assert!(logs.starts_with("Error in backend to client send messages: "));
}

#[tokio::test]
async fn missing_client_ends_the_relay_with_1011() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text("not json");
    let options = StateOptions {
        sockets: Sockets::BackendOnly,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    let close = result.unwrap();
    assert_eq!(close.code, 1011);
    assert_eq!(
        close.reason,
        "proxy failed while relaying the upstream websocket"
    );
}

#[tokio::test]
async fn receive_failure_flushes_unbilled_usage_and_logs_success() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    fixture.provider.unbilled_seconds = Some(2);
    let mut source = fixture.source.clone();
    source.fail_receive(0);
    let options = StateOptions {
        provider: Flag(true),
        logging: Logging::LoggerAndWorker,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1011);
    assert_eq!(fixture.logging.dispatches(), 1);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from(COMPLETED)]
    );
    assert!(state.messages[0].get("usage").is_some());
    assert!(fixture.dispatch.refusals().is_empty());
}
