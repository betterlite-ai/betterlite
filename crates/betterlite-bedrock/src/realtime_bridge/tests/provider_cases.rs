use betterlite_models::decisions::Json;

use super::support::{self, Chunk, ProviderFixture, ScriptedClient, ScriptedStream};
use crate::realtime_bridge::end::ForwardEnd;
use crate::realtime_bridge::error::ProviderForwardError;

const CONTENT_START: &str = r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;
const HELLO: &str = r#"{"event":{"textOutput":{"content":"Hello"}}}"#;
const WORLD: &str = r#"{"event":{"textOutput":{"content":" world"}}}"#;
const END_TURN: &str = r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#;

pub(super) async fn provider_event_forwarding() {
    let stream = ScriptedStream::new(vec![
        Chunk::bytes(r#"{"event":{"sessionStart":{}}}"#),
        Chunk::bytes(CONTENT_START),
        Chunk::MissingValue,
        Chunk::MissingBytes,
        Chunk::Bytes(Vec::new()),
        Chunk::bytes(HELLO),
        Chunk::bytes(WORLD),
        Chunk::bytes(END_TURN),
        Chunk::End,
    ]);
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    fixture.state.current_item_chunks = Some(Vec::new());
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("the provider pump ends normally"),
        ForwardEnd::Ended
    );
    assert_eq!(fixture.client.closes(), vec![(1000, None)]);
    let sent = fixture.client.sent_events();
    let kinds: Vec<Option<&str>> = sent.iter().map(event_type).collect();
    assert_eq!(
        kinds,
        [
            Some("response.created"),
            Some("response.output_item.added"),
            Some("response.content_part.added"),
            Some("response.text.delta"),
            Some("response.text.delta"),
            Some("response.text.done"),
            Some("response.content_part.done"),
            Some("response.output_item.done"),
            Some("response.done"),
        ]
    );
    assert_eq!(event_text(&sent[5], "text"), Some("Hello world"));
    // The end turn resets the response, its item and the delta type; the conversation stays for the next one
    assert_eq!(fixture.state.current_response_id, None);
    assert_eq!(fixture.state.current_output_item_id, None);
    assert_eq!(
        fixture.state.current_conversation_id,
        Some(format!("conv_{}", support::numbered(6)))
    );
    assert_eq!(fixture.state.current_delta_type, None);
    assert_eq!(fixture.state.current_delta_chunks, None);
    assert_eq!(
        fixture.state.session_configuration_request.as_deref(),
        Some(r#"{"configured": true}"#)
    );
    assert_eq!(fixture.state.current_item_chunks, Some(Vec::new()));

    assert_eq!(fixture.logged.len(), 1);
    assert_eq!(event_type(&fixture.logged[0]), Some("response.done"));
}

pub(super) async fn provider_error_close() {
    await_failure();
    receive_failure();
    utf8_failure();
    transform_failure();
    client_send_failure();
}

async fn await_failure() {
    let stream = ScriptedStream::new(Vec::new()).with_await_failure("the provider has no output");
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    let outcome = fixture.run().await;

    assert!(matches!(outcome, Err(ProviderForwardError::Stream(_))));
    assert!(fixture.client.sent_events().is_empty());
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.logged.is_empty());
    assert_eq!(fixture.state, support::input());
}

async fn receive_failure() {
    let stream = ScriptedStream::new(vec![Chunk::bytes(CONTENT_START)]).with_receive_failure(1);
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    let outcome = fixture.run().await;

    assert!(matches!(outcome, Err(ProviderForwardError::Stream(_))));
    assert_eq!(fixture.client.sent_events().len(), 3);
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.state.current_response_id.is_some());
    assert!(fixture.logged.is_empty());
}

async fn utf8_failure() {
    let stream = ScriptedStream::new(vec![Chunk::Bytes(vec![0xff])]);
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    let outcome = fixture.run().await;

    assert!(matches!(outcome, Err(ProviderForwardError::Utf8 { .. })));
    assert!(fixture.client.sent_events().is_empty());
    assert!(fixture.client.closes().is_empty());
}

async fn transform_failure() {
    let stream = ScriptedStream::new(vec![Chunk::bytes(r#"{"event":{"contentStart":[]}}"#)]);
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    let outcome = fixture.run().await;

    assert!(matches!(outcome, Err(ProviderForwardError::Transform(_))));
    assert!(fixture.client.sent_events().is_empty());
    assert!(fixture.client.closes().is_empty());
    assert_eq!(fixture.state, support::input());
}

async fn client_send_failure() {
    let stream = ScriptedStream::new(vec![
        Chunk::bytes(CONTENT_START),
        Chunk::bytes(END_TURN),
        Chunk::End,
    ]);
    let client = ScriptedClient::new(Vec::new()).failing_send(0);
    let mut fixture = ProviderFixture::new(client, stream);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("a client send failure ends forwarding"),
        ForwardEnd::Ended
    );
    assert_eq!(fixture.client.sent_events().len(), 1);
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.logged.is_empty());
    // The first content start already updated the shared state; the failure does not roll it back
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

fn event_text(event: &Json, field: &str) -> Option<&str> {
    match event {
        Json::Object(fields) => match fields.get(field) {
            Some(Json::Text(text)) => Some(text.as_str()),
            _ => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
