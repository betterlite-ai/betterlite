use betterlite_models::decisions::Json;

use super::support::{
    Chunk, ClientFixture, ProviderFixture, ScriptedClient, ScriptedStream, event_key,
};
use crate::realtime_bridge::end::ForwardEnd;
use crate::realtime_bridge::error::ClientForwardError;

const SESSION_UPDATE: &str = r#"{"type": "session.update", "session": {}}"#;
const CONTENT_START: &str = r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;

pub(super) async fn input_half_close() {
    without_prompt_started().await;
    close_frames_fail_individually().await;
    one_close_frame_failure_keeps_the_rest().await;
    close_failure_keeps_the_original_error().await;
    readiness_failure_keeps_the_replay_marker().await;
}

async fn without_prompt_started() {
    let mut fixture = ClientFixture::new(None, ScriptedClient::new(vec![None]));
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("a client receive failure ends the pump"),
        ForwardEnd::Ended
    );
    assert!(fixture.stream.sent().is_empty());
    assert!(fixture.stream.closed());
    assert_eq!(fixture.scope.pending_session_update, Json::Null);
}

async fn close_frames_fail_individually() {
    let mut fixture = ClientFixture::new(
        Some(SESSION_UPDATE.to_owned()),
        ScriptedClient::new(vec![None]),
    );
    fixture.stream = ScriptedStream::new(Vec::new()).with_send_failures(&[2, 3]);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("closing failures stay suppressed"),
        ForwardEnd::Ended
    );
    let frames = fixture.stream.sent();
    let keys: Vec<Option<&str>> = frames.iter().map(event_key).collect();
    assert_eq!(
        keys,
        [
            Some("sessionStart"),
            Some("promptStart"),
            Some("promptEnd"),
            Some("sessionEnd"),
        ]
    );
    assert!(fixture.stream.closed());
}

async fn one_close_frame_failure_keeps_the_rest() {
    let mut fixture = ClientFixture::new(
        Some(SESSION_UPDATE.to_owned()),
        ScriptedClient::new(vec![None]),
    );
    fixture.stream = ScriptedStream::new(Vec::new()).with_send_failures(&[2]);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("one closing failure never skips the next frame"),
        ForwardEnd::Ended
    );
    assert_eq!(fixture.stream.sent().len(), 4);
    assert!(fixture.stream.closed());
}

async fn close_failure_keeps_the_original_error() {
    let mut fixture = ClientFixture::new(
        Some(SESSION_UPDATE.to_owned()),
        ScriptedClient::new(vec![None]),
    );
    fixture.stream = ScriptedStream::new(Vec::new())
        .with_send_failures(&[1, 2, 3])
        .with_close_failure();
    let outcome = fixture.run().await;

    assert!(matches!(outcome, Err(ClientForwardError::Send(_))));
    assert!(fixture.stream.closed());
    let frames = fixture.stream.sent();
    assert_eq!(frames.len(), 4);
    // The session.update never reached an ack, so the exact raw message stays for a replay
    assert_eq!(
        fixture.scope.pending_session_update,
        Json::Text(SESSION_UPDATE.to_owned())
    );
    assert_eq!(fixture.scope.session_committed, Json::Null);
    assert!(fixture.client.sent_events().is_empty());
}

async fn readiness_failure_keeps_the_replay_marker() {
    let mut fixture = ClientFixture::new(
        Some(SESSION_UPDATE.to_owned()),
        ScriptedClient::new(vec![None]),
    );
    fixture.stream =
        ScriptedStream::new(Vec::new()).with_await_failure("the provider stream never opened");
    let outcome = fixture.run().await;

    assert!(matches!(outcome, Err(ClientForwardError::Ack(_))));
    assert_eq!(
        fixture.scope.pending_session_update,
        Json::Text(SESSION_UPDATE.to_owned())
    );
    assert_eq!(fixture.scope.session_committed, Json::Null);
    assert!(fixture.client.sent_events().is_empty());
    assert!(fixture.stream.closed());
}

pub(super) async fn output_half_close() {
    eof_without_events().await;
    eof_after_unknown_event().await;
    eof_after_transformed_events().await;
}

fn provider_fixture(chunks: Vec<Chunk>) -> ProviderFixture {
    ProviderFixture::new(ScriptedClient::new(Vec::new()), ScriptedStream::new(chunks))
}

async fn eof_without_events() {
    let mut fixture = provider_fixture(vec![Chunk::End]);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("a normal provider end returns"),
        ForwardEnd::Ended
    );
    assert!(fixture.client.sent_events().is_empty());
    assert_eq!(fixture.client.closes(), vec![(1000, None)]);
    assert!(fixture.log.events.is_empty());
}

async fn eof_after_unknown_event() {
    let mut fixture = provider_fixture(vec![
        Chunk::bytes(r#"{"event":{"unknownThing":1}}"#),
        Chunk::End,
    ]);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("an unknown provider event is not an error"),
        ForwardEnd::Ended
    );
    assert!(fixture.client.sent_events().is_empty());
    assert_eq!(fixture.client.closes(), vec![(1000, None)]);
}

async fn eof_after_transformed_events() {
    let mut fixture = provider_fixture(vec![Chunk::bytes(CONTENT_START), Chunk::End]);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("eof after events returns"),
        ForwardEnd::Ended
    );
    let sent = fixture.client.sent_events();
    assert_eq!(sent.len(), 3);
    assert!(
        sent.iter()
            .all(|event| event_type(event) != Some("response.done"))
    );
    assert_eq!(fixture.client.closes(), vec![(1000, None)]);
    assert!(fixture.log.events.is_empty());
    assert!(fixture.state.current_response_id.is_some());
}

fn event_type(event: &Json) -> Option<&str> {
    match event {
        Json::Object(fields) => match fields.get("type") {
            Some(Json::Text(text)) => Some(text.as_str()),
            _ => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
