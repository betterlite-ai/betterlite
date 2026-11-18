use betterlite_models::decisions::Json;

use super::support::{
    Chunk, ClientFixture, ProviderFixture, ScriptedClient, ScriptedStream, event_key,
};
use crate::realtime_bridge::end::ForwardEnd;

const SESSION_UPDATE: &str = r#"{"type":"session.update","session":{}}"#;
const CONTENT_START: &str = r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;
const END_TURN: &str = r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#;

pub(super) async fn cancel_session() {
    client_pump_closes_input().await;
    provider_pump_leaves_client_open().await;
}

async fn client_pump_closes_input() {
    let mut fixture =
        ClientFixture::new(Some(SESSION_UPDATE.to_owned()), ScriptedClient::blocking());
    let outcome = fixture.run_cancelled(1).await;

    assert_eq!(
        outcome.expect("the cancelled client pump returns"),
        ForwardEnd::Cancelled
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
    // The ack for the initial session.update left before the pump blocked on the next client frame
    assert_eq!(fixture.client.sent_events().len(), 1);
    assert!(fixture.client.closes().is_empty());
}

async fn provider_pump_leaves_client_open() {
    let stream =
        ScriptedStream::new(vec![Chunk::bytes(CONTENT_START), Chunk::bytes(END_TURN)]).blocking();
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    // Cancelled while the third receive blocks, after the first response.done was sent and logged
    let outcome = fixture.run_cancelled(3).await;

    assert_eq!(
        outcome.expect("the cancelled provider pump returns"),
        ForwardEnd::Cancelled
    );
    assert_eq!(fixture.client.sent_events().len(), 7);
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.state.current_conversation_id.is_some());
    assert!(!fixture.stream.closed());
    // A pump that kept events until it returned would have lost this one to the cancellation
    assert_eq!(fixture.log.events.len(), 1);
    assert_eq!(event_type(&fixture.log.events[0]), Some("response.done"));
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
