use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::raw::parse_backend_event;

use super::harness::*;

#[tokio::test]
async fn backend_frame_decoding() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"vendor.event","n":1}"#);
    source.binary(br#"{"type":"conversation.item.added","item":{"id":"i"}}"#.to_vec());
    source.text("not json");
    source.text("[]");
    source.close_with(1008, "policy");
    let options = StateOptions::default()
        .normalizer(normalizer(&[], &[("vendor.event", "vendor.event.v2")]))
        .logged_event_types(LoggedEventTypes::All);
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1008);
    assert!(state.backend_sent_frames.0);
    assert_eq!(
        fixture.client.frames(),
        vec![
            String::from(r#"{"type":"vendor.event.v2","n":1}"#),
            String::from(r#"{"type":"conversation.item.added","item":{"id":"i"}}"#),
            String::from("not json"),
            String::from("[]"),
        ]
    );
    assert_eq!(
        message_types(&state.messages),
        vec![
            String::from("vendor.event.v2"),
            String::from("conversation.item.added")
        ]
    );
}

#[tokio::test]
async fn backend_frame_decoding_drops_normalized_events() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"response.done","response":{}}"#);
    source.close_with(1000, "");
    let options = StateOptions::default()
        .normalizer(normalizer(&["response.done"], &[]))
        .logged_event_types(LoggedEventTypes::All);
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(fixture.client.frames().is_empty());
    assert!(state.messages.is_empty());
}

#[tokio::test]
async fn backend_frame_decoding_retries_without_the_decode_argument() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.reject_decode(0);
    source.text(r#"{"type":"vendor.event","n":1}"#);
    source.close_with(1000, "");
    let token = CancellationToken::new();
    let (result, _state) = run(&StateOptions::default(), &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(source.decode_calls(), vec![Some(false), None, Some(false)]);
    assert_eq!(
        fixture.client.frames(),
        vec![String::from(r#"{"type":"vendor.event","n":1}"#)]
    );
}

#[tokio::test]
async fn skipped_binary_frame_still_marks_frames_sent() {
    let (logs, _guard) = capture();
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.binary(vec![0xff, 0xfe]);
    source.close_with(1008, "policy");
    let options = StateOptions::default().logging(Logging::LoggerAndWorker);
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1008);
    assert!(state.backend_sent_frames.0);
    assert!(fixture.dispatch.refusals().is_empty());
    assert_eq!(fixture.logging.dispatches(), 1);
    assert!(logs.contains("Received non-UTF-8 binary frame from backend, skipping."));
}

#[tokio::test]
async fn raw_beta_client_gets_translated_events() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(
        r#"{"type":"conversation.item.added","item":{"content":[{"type":"output_text","text":"hi"}]}}"#,
    );
    source.text(r#"{"type":"conversation.item.done","item":{"id":"i"}}"#);
    source.close_with(1000, "");
    let options = StateOptions::default()
        .client_wants_beta()
        .logged_event_types(LoggedEventTypes::All);
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        fixture.client.frames(),
        vec![String::from(
            r#"{"type":"conversation.item.created","item":{"content":[{"type":"text","text":"hi"}]}}"#
        )]
    );
    assert_eq!(
        message_types(&state.messages),
        vec![
            String::from("conversation.item.added"),
            String::from("conversation.item.done")
        ]
    );
}

#[test]
fn parse_backend_event_keeps_only_objects() {
    assert!(parse_backend_event("not json").is_none());
    assert!(parse_backend_event("[1]").is_none());
    assert!(parse_backend_event("null").is_none());
    assert!(parse_backend_event("\"text\"").is_none());
    assert!(parse_backend_event("1").is_none());
    assert_eq!(
        parse_backend_event(r#"{"type":"x"}"#),
        Some(json(r#"{"type":"x"}"#))
    );
}
