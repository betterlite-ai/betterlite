use super::support::{
    Chunk, ClientFixture, ProviderFixture, ScriptedClient, ScriptedStream, event_key,
};
use crate::realtime_bridge::end::ForwardEnd;

const SESSION_UPDATE: &str = r#"{"type":"session.update","session":{}}"#;
const CONTENT_START: &str = r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;

pub(super) async fn cancel_session() {
    client_pump_closes_input();
    provider_pump_leaves_client_open();
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
    let stream = ScriptedStream::new(vec![Chunk::bytes(CONTENT_START)]).blocking();
    let mut fixture = ProviderFixture::new(ScriptedClient::new(Vec::new()), stream);
    let outcome = fixture.run_cancelled(2).await;

    assert_eq!(
        outcome.expect("the cancelled provider pump returns"),
        ForwardEnd::Cancelled
    );
    assert_eq!(fixture.client.sent_events().len(), 3);
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.state.current_response_id.is_some());
    assert!(!fixture.stream.closed());
}
