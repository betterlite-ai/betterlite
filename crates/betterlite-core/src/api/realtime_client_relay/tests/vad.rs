use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::close::ClientLoopExit;

use super::harness::*;

fn session_update(session: &str) -> String {
    format!(r#"{{"type":"session.update","session":{session}}}"#)
}

fn transcription_guardrail() -> ScriptedGuardrail {
    ScriptedGuardrail::allowing(vec![GuardrailEventHooks::RealtimeInputTranscription])
}

fn backend_sessions(outcome: &Outcome) -> Vec<Json> {
    outcome
        .backend
        .parsed()
        .iter()
        .map(|frame| match frame {
            Json::Object(fields) => fields.get("session").cloned().unwrap_or(Json::Null),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Json::Null
            }
        })
        .collect()
}

#[tokio::test]
async fn client_guardrail_vad_cannot_be_reenabled() {
    first_update_injects_before_the_ga_remap().await;
    beta_backend_keeps_the_flat_shape().await;
    later_updates_force_the_flag_false().await;
    absent_shapes_receive_a_flat_disable().await;
    without_transcription_guardrails_values_survive().await;
    the_flag_is_marked_only_after_a_delivery().await;
    the_provider_patch_runs_last().await;
    the_patch_refreshes_a_beta_session().await;
}

async fn first_update_injects_before_the_ga_remap() {
    let outcome = ack(Fixture::default()
        .with_guardrail(transcription_guardrail())
        .with_client_message(&session_update(
            r#"{"turn_detection":{"type":"server_vad","silence_duration_ms":700}}"#,
        )))
    .await;
    assert!(outcome.state.setup.turn_detection_update_sent.0);
    assert_eq!(
        backend_sessions(&outcome),
        vec![json(
            r#"{"type":"realtime","audio":{"input":{"turn_detection":{"type":"server_vad","silence_duration_ms":700,"create_response":false}}}}"#
        )]
    );
}

async fn beta_backend_keeps_the_flat_shape() {
    let outcome = ack(Fixture::default()
        .with_guardrail(transcription_guardrail())
        .with_beta_backend()
        .with_client_message(&session_update(
            r#"{"turn_detection":{"silence_duration_ms":700}}"#,
        )))
    .await;
    assert_eq!(
        backend_sessions(&outcome),
        vec![json(
            r#"{"turn_detection":{"silence_duration_ms":700,"create_response":false}}"#
        )]
    );
}

async fn later_updates_force_the_flag_false() {
    let outcome = ack(Fixture::default()
        .with_guardrail(transcription_guardrail())
        .with_turn_detection_sent(true)
        .with_beta_backend()
        .with_client_message(&session_update(
            r#"{"turn_detection":{"create_response":true,"silence_duration_ms":500}}"#,
        ))
        .with_client_message(&session_update(
            r#"{"audio":{"input":{"turn_detection":{"create_response":true,"x":1}}}}"#,
        ))
        .with_client_message(&session_update(r#"{"turn_detection":"vad"}"#))
        .with_client_message(&session_update(r#"{"turn_detection":null}"#)))
    .await;
    assert!(outcome.state.setup.turn_detection_update_sent.0);
    assert_eq!(
        backend_sessions(&outcome),
        vec![
            json(r#"{"turn_detection":{"create_response":false,"silence_duration_ms":500}}"#),
            json(r#"{"audio":{"input":{"turn_detection":{"create_response":false,"x":1}}}}"#),
            json(r#"{"turn_detection":{"create_response":false}}"#),
            json(r#"{"turn_detection":{"create_response":false}}"#),
        ]
    );
}

async fn absent_shapes_receive_a_flat_disable() {
    let outcome = ack(Fixture::default()
        .with_guardrail(transcription_guardrail())
        .with_turn_detection_sent(true)
        .with_client_message(&session_update("{}")))
    .await;
    assert_eq!(
        backend_sessions(&outcome),
        vec![json(
            r#"{"type":"realtime","audio":{"input":{"turn_detection":{"create_response":false}}}}"#
        )]
    );
}

async fn without_transcription_guardrails_values_survive() {
    let outcome = ack(Fixture::default().with_client_message(&session_update(
        r#"{"turn_detection":{"create_response":true,"silence_duration_ms":700}}"#,
    )))
    .await;
    assert!(!outcome.state.setup.turn_detection_update_sent.0);
    assert_eq!(
        backend_sessions(&outcome),
        vec![json(
            r#"{"type":"realtime","audio":{"input":{"turn_detection":{"create_response":true,"silence_duration_ms":700}}}}"#
        )]
    );
}

async fn the_flag_is_marked_only_after_a_delivery() {
    let dropped = ack(Fixture::default()
        .with_provider(ScriptedProvider {
            requires_session_configuration: false,
            delivers: false,
        })
        .with_setup_complete(true)
        .with_guardrail(transcription_guardrail())
        .with_client_message(&session_update(r#"{"instructions":"hi"}"#)))
    .await;
    assert!(!dropped.state.setup.turn_detection_update_sent.0);
    assert!(dropped.backend.frames().is_empty());
    let failed = ack(Fixture::default()
        .with_guardrail(transcription_guardrail())
        .with_backend_failures(vec![0])
        .with_client_message(&session_update(r#"{"instructions":"hi"}"#)))
    .await;
    assert_eq!(failed.exit, ClientLoopExit::BackendClosed);
    assert!(!failed.state.setup.turn_detection_update_sent.0);
}

async fn the_patch_refreshes_a_beta_session() {
    let outcome = ack(Fixture::default()
        .with_beta_backend()
        .with_normalizer()
        .with_client_message(&session_update(r#"{"instructions":"hi"}"#)))
    .await;
    assert_eq!(
        backend_sessions(&outcome),
        vec![json(r#"{"instructions":"hi","xai_patch":true}"#)]
    );
}

async fn the_provider_patch_runs_last() {
    let outcome = ack(Fixture::default()
        .with_guardrail(transcription_guardrail())
        .with_normalizer()
        .with_client_message(&session_update(
            r#"{"turn_detection":{"silence_duration_ms":700}}"#,
        )))
    .await;
    let patched = outcome.patched_sessions();
    assert_eq!(patched.len(), 1);
    let Json::Object(fields) = &patched[0] else {
        panic!("the normalizer receives the session object");
    };
    assert_eq!(fields.get("xai_patch"), Some(&Json::Bool(true)));
    assert_eq!(
        fields.get("audio"),
        Some(&json(
            r#"{"input":{"turn_detection":{"silence_duration_ms":700,"create_response":false}}}"#
        ))
    );
    assert_eq!(
        backend_sessions(&outcome),
        vec![json(
            r#"{"type":"realtime","audio":{"input":{"turn_detection":{"silence_duration_ms":700,"create_response":false}}},"xai_patch":true}"#
        )]
    );
}
