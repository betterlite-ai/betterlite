use std::convert::Infallible;
use std::sync::Arc;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::RESPONSE_CREATE;
use crate::api::realtime_contracts::events::AllDeltaType;
use crate::api::realtime_guardrails::callback::RealtimeGuardrail;

use super::harness::*;

#[tokio::test]
async fn transformed_session_ready() {
    let fixture = Fixture::default();
    respond(&fixture, json(SESSION_CREATED));
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        pending_messages: vec![String::from("audio-1"), String::from("audio-2")],
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.setup.backend_setup_complete.0);
    assert!(!state.setup.flushing_pending_messages.0);
    assert!(state.setup.pending_messages_until_setup.is_empty());
    assert!(state.setup.session_created_sent_to_client.0);
    assert_eq!(
        lock(&fixture.provider.script).requests,
        vec![String::from("audio-1"), String::from("audio-2")]
    );
    assert_eq!(fixture.client.parsed(), vec![json(SESSION_CREATED)]);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from("session.created")]
    );
}

#[tokio::test]
async fn transformed_session_list_result_keeps_event_order() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(
        &fixture,
        Json::List(vec![
            json(SESSION_CREATED),
            json(r#"{"type":"response.output_text.delta","delta":"hi"}"#),
        ]),
    );
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        logged_event_types: LoggedEventTypes::All,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        fixture.client.parsed(),
        vec![
            json(SESSION_CREATED),
            json(r#"{"type":"response.output_text.delta","delta":"hi"}"#)
        ]
    );
    assert_eq!(
        message_types(&state.messages),
        vec![
            String::from("session.created"),
            String::from("response.output_text.delta")
        ]
    );
}

#[tokio::test]
async fn transformed_session_ready_updates_transform_input() {
    let fixture = Fixture::default();
    let mut script = lock(&fixture.provider.script);
    script.responses.push_back(ScriptedResponse {
        response: json(r#"{"type":"a"}"#),
        current_output_item_id: Some(String::from("out-1")),
        current_response_id: Some(String::from("resp-1")),
        current_conversation_id: Some(String::from("conv-1")),
        current_delta_type: Some(AllDeltaType::Text),
        session_configuration_request: Some(String::from("cfg-1")),
        ..ScriptedResponse::default()
    });
    drop(script);
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    let inputs = lock(&fixture.provider.script).inputs.clone();
    assert_eq!(
        (
            inputs[1].current_output_item_id.as_deref(),
            inputs[1].current_response_id.as_deref(),
            inputs[1].current_conversation_id.as_deref(),
            inputs[1].session_configuration_request.as_deref(),
        ),
        (Some("out-1"), Some("resp-1"), Some("conv-1"), Some("cfg-1"))
    );
    assert_eq!(inputs[1].current_delta_type, Some(AllDeltaType::Text));
    assert!(inputs[0].current_output_item_id.is_none());
}

#[tokio::test]
async fn transformed_dropped_events_are_not_forwarded() {
    let fixture = Fixture::default();
    respond(
        &fixture,
        Json::List(vec![
            json(r#"{"type":"vendor.drop"}"#),
            json(r#"{"type":"vendor.keep"}"#),
        ]),
    );
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        normalizer: Some(normalizer(&["vendor.drop"], &[])),
        logged_event_types: LoggedEventTypes::All,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        fixture.client.parsed(),
        vec![json(r#"{"type":"vendor.keep"}"#)]
    );
    assert_eq!(
        message_types(&state.messages),
        vec![String::from("vendor.keep")]
    );
}

#[tokio::test]
async fn transformed_session_created_without_pending_does_not_flush() {
    let fixture = Fixture::default();
    respond(&fixture, json(SESSION_CREATED));
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.setup.backend_setup_complete.0);
    assert!(!state.setup.flushing_pending_messages.0);
    assert!(state.setup.pending_messages_until_setup.is_empty());
    assert!(lock(&fixture.provider.script).requests.is_empty());
    assert_eq!(fixture.client.parsed(), vec![json(SESSION_CREATED)]);
}

#[tokio::test]
async fn transformed_session_created_skips_flush_when_the_provider_configures_the_session() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(&fixture, json(SESSION_CREATED));
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        pending_messages: vec![String::from("audio-1")],
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.setup.backend_setup_complete.0);
    assert!(!state.setup.flushing_pending_messages.0);
    assert!(lock(&fixture.provider.script).requests.is_empty());
    assert_eq!(
        state.setup.pending_messages_until_setup,
        vec![String::from("audio-1")]
    );
    assert_eq!(state.setup.pending_messages_byte_total, 7);
}

#[tokio::test]
async fn transformed_flush_failure_retains_frames_and_resets_flushing() {
    let fixture = Fixture::default();
    {
        let mut script = lock(&fixture.provider.script);
        script.responses.push_back(ScriptedResponse {
            response: json(SESSION_CREATED),
            ..ScriptedResponse::default()
        });
        script.request_failures.push(0);
    }
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        pending_messages: vec![String::from("a"), String::from("b")],
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.setup.backend_setup_complete.0);
    assert!(!state.setup.flushing_pending_messages.0);
    assert_eq!(
        state.setup.pending_messages_until_setup,
        vec![String::from("a"), String::from("b")]
    );
    assert_eq!(state.setup.pending_messages_byte_total, 2);
    assert_eq!(
        lock(&fixture.provider.script).requests,
        vec![String::from("a")]
    );
    assert_eq!(fixture.client.parsed(), vec![json(SESSION_CREATED)]);
}

#[tokio::test]
async fn transformed_session_created_sends_disable_after_forwarding() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(&fixture, json(SESSION_CREATED));
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        ..StateOptions::default()
    };
    let guardrail = ScriptedGuardrail::default();
    let records = Arc::clone(&guardrail.records);
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &guardrails, &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.setup.turn_detection_update_sent.0);
    assert_eq!(fixture.client.parsed(), vec![json(SESSION_CREATED)]);
    assert_eq!(fixture.backend.parsed(), vec![json(DISABLE)]);
    let requests = lock(&fixture.provider.script).requests.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(json(&requests[0]), json(DISABLE));
    assert!(
        lock(&records)
            .checks
            .contains(&GuardrailEventHooks::RealtimeInputTranscription)
    );
}

#[tokio::test]
async fn transformed_duplicate_session_created_retries_a_failed_disable() {
    let (logs, _guard) = capture();
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(&fixture, json(SESSION_CREATED));
    respond(&fixture, json(SESSION_CREATED));
    lock(&fixture.provider.script).request_failures.push(0);
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame","n":1}"#);
    source.text(r#"{"type":"provider.frame","n":2}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        ..StateOptions::default()
    };
    let guardrail = ScriptedGuardrail::default();
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &guardrails, &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(state.setup.turn_detection_update_sent.0);
    assert_eq!(
        fixture.client.parsed(),
        vec![
            json(SESSION_CREATED),
            json(r#"{"type":"session.updated","session":{"id":"s1"}}"#)
        ]
    );
    assert_eq!(fixture.backend.parsed(), vec![json(DISABLE)]);
    assert_eq!(lock(&fixture.provider.script).requests.len(), 2);
    assert!(logs.starts_with("Error processing backend message, skipping: "));
}

#[tokio::test]
async fn transformed_client_send_failure_keeps_updated_state() {
    let (logs, _guard) = capture();
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    let mut script = lock(&fixture.provider.script);
    script.responses.push_back(ScriptedResponse {
        response: json(r#"{"type":"response.output_text.delta","delta":"hi"}"#),
        current_output_item_id: Some(String::from("out-9")),
        current_response_id: Some(String::from("resp-9")),
        ..ScriptedResponse::default()
    });
    drop(script);
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        logged_event_types: LoggedEventTypes::All,
        client_failures: vec![0],
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        (
            state.response.current_output_item_id.as_deref(),
            state.response.current_response_id.as_deref()
        ),
        (Some("out-9"), Some("resp-9"))
    );
    assert_eq!(
        message_types(&state.messages),
        vec![String::from("response.output_text.delta")]
    );
    assert!(fixture.client.frames().is_empty());
    assert!(logs.starts_with("Error processing backend message, skipping: "));
}

#[tokio::test]
async fn transformed_transcription_session_still_runs_guardrails() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(&fixture, completed_json());
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        force_transcription_model: Some(String::from("gpt-realtime-whisper")),
        ..StateOptions::default()
    };
    let guardrail = ScriptedGuardrail::default();
    let records = Arc::clone(&guardrail.records);
    let guardrails: [&dyn RealtimeGuardrail<Error = Infallible>; 1] = [&guardrail];
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &guardrails, &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(!lock(&records).checks.is_empty());
    assert_eq!(lock(&records).applied.len(), 1);
    assert!(fixture.backend.frames().is_empty());
    assert!(lock(&fixture.provider.script).requests.is_empty());
    assert_eq!(
        message_types(&state.messages),
        vec![String::from(COMPLETED)]
    );
    assert!(state.messages[0].get("usage").is_some());
    assert_eq!(state.input_messages.len(), 1);
    assert_eq!(fixture.client.parsed(), vec![completed_json()]);
}

#[tokio::test]
async fn transformed_conversational_turn_asks_for_a_response() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(&fixture, completed_json());
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, _state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert_eq!(
        lock(&fixture.provider.script).requests,
        vec![String::from(RESPONSE_CREATE)]
    );
    assert_eq!(
        fixture.backend.frames(),
        vec![String::from(RESPONSE_CREATE)]
    );
}

#[tokio::test]
async fn transformed_session_events_do_not_mark_transcription_mode() {
    let mut fixture = Fixture::default();
    fixture.provider.requires_session_configuration = true;
    respond(
        &fixture,
        json(r#"{"type":"transcription_session.created","session":{}}"#),
    );
    let mut source = fixture.source.clone();
    source.text(r#"{"type":"provider.frame"}"#);
    source.close_with(1000, "");
    let options = StateOptions {
        provider: Flag(true),
        logged_event_types: LoggedEventTypes::All,
        ..StateOptions::default()
    };
    let token = CancellationToken::new();
    let (result, state) = run(&options, &fixture, &mut source, &[], &token).await;
    assert_eq!(result.unwrap().code, 1000);
    assert!(!state.is_transcription_session.0);
    assert_eq!(
        message_types(&state.messages),
        vec![String::from("transcription_session.created")]
    );
}
