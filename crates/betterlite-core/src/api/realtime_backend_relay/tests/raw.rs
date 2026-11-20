use std::convert::Infallible;
use std::sync::Arc;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::RESPONSE_CREATE;
use crate::api::realtime_guardrails::callback::RealtimeGuardrail;

use super::harness::*;

#[tokio::test]
async fn backend_transcription_turns() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(COMPLETED_FRAME);
    source.close_with(1000, "");
    let options = StateOptions::default()
        .logged_event_types(LoggedEventTypes::All)
        .logging(Logging::LoggerAndWorker);
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        fixture.backend.frames(),
        vec![String::from(RESPONSE_CREATE)]
    );
    assert_eq!(fixture.client.parsed(), vec![completed_json()]);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from(COMPLETED)]
    );
    assert_eq!(
        state.input_messages,
        vec![IndexMap::from([
            (String::from("role"), Json::Text(String::from("user"))),
            (String::from("content"), Json::Text(String::from("hello"))),
        ])]
    );
    assert_eq!(fixture.logging.dispatches(), 1);
}

#[tokio::test]
async fn raw_transcription_session_captures_usage_and_skips_the_turn() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(COMPLETED_FRAME);
    source.close_with(1000, "");
    let options = StateOptions::default().force_transcription_model("gpt-realtime-whisper");
    let guardrail = ScriptedGuardrail::default();
    let records = Arc::clone(&guardrail.records);
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &guardrails, &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.is_transcription_session.0);
    assert!(fixture.backend.frames().is_empty());
    assert_eq!(fixture.client.parsed(), vec![completed_json()]);
    assert!(state.messages[0].get("usage").is_some());
    assert!(lock(&records).checks.is_empty());
}

#[tokio::test]
async fn raw_blocked_transcript_sends_no_clean_turn() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(COMPLETED_FRAME);
    source.close_with(1000, "");
    let guardrail = ScriptedGuardrail::blocking();
    let records = Arc::clone(&guardrail.records);
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, _state) = run(
        &StateOptions::default(),
        &fixture,
        &mut source,
        &guardrails,
        &token,
    )
    .await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        json_types(&fixture.backend.parsed()),
        vec![
            String::from("response.cancel"),
            String::from("conversation.item.create"),
            String::from("response.create")
        ]
    );
    assert_eq!(
        json_types(&fixture.client.parsed()),
        vec![String::from(COMPLETED), String::from("error")]
    );
    assert_eq!(lock(&records).applied.len(), 1);
}

#[tokio::test]
async fn raw_session_update_marks_a_transcription_session() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"session.created","session":{"id":"s1","type":"transcription"}}"#);
    source.text(COMPLETED_FRAME);
    source.close_with(1000, "");
    let token = CancellationToken::new();
    let (result, state) = run(&StateOptions::default(), &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.is_transcription_session.0);
    assert!(fixture.backend.frames().is_empty());
    assert_eq!(fixture.client.parsed().len(), 2);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from("session.created"), String::from(COMPLETED)]
    );
}

#[tokio::test]
async fn raw_transcription_session_created_marks_the_mode() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    let created = r#"{"type":"transcription_session.created","session":{"id":"s1"}}"#;
    source.text(created);
    source.close_with(1000, "");
    let options = StateOptions::default().logged_event_types(LoggedEventTypes::All);
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.is_transcription_session.0);
    assert_eq!(fixture.client.parsed(), vec![json(created)]);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from("transcription_session.created")]
    );
}

#[tokio::test]
async fn raw_guardrail_receives_the_completed_transcript() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(COMPLETED_FRAME);
    source.close_with(1000, "");
    let guardrail = ScriptedGuardrail::default();
    let records = Arc::clone(&guardrail.records);
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, _state) = run(
        &StateOptions::default(),
        &fixture,
        &mut source,
        &guardrails,
        &token,
    )
    .await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(lock(&records).applied.len(), 1);
    assert_eq!(lock(&records).applied[0].texts, vec![String::from("hello")]);
}

#[tokio::test]
async fn raw_guardrailed_session_created_disables_on_every_event() {
    let fixture = Fixture::default();
    let mut source = fixture.source.clone();
    source.text(SESSION_CREATED);
    source.text(SESSION_CREATED);
    source.close_with(1000, "");
    let guardrail = ScriptedGuardrail::default();
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, state) = run(
        &StateOptions::default(),
        &fixture,
        &mut source,
        &guardrails,
        &token,
    )
    .await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        fixture.client.parsed(),
        vec![json(SESSION_CREATED), json(SESSION_CREATED)]
    );
    assert_eq!(fixture.backend.parsed(), vec![json(DISABLE), json(DISABLE)]);
    assert!(!state.setup.turn_detection_update_sent.0);
    assert!(!state.setup.session_created_sent_to_client.0);
}
