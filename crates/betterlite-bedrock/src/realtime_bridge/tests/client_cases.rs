use betterlite_models::decisions::Json;

use super::support::{self, ClientFixture, ScriptedClient, event_key, json};
use crate::realtime_bridge::end::ForwardEnd;

const SESSION_UPDATE: &str = r#"{"type": "session.update", "session": {}}"#;
const AUDIO_APPEND: &str = r#"{"type":"input_audio_buffer.append","audio":"AAAA"}"#;
const AUDIO_COMMIT: &str = r#"{"type":"input_audio_buffer.commit"}"#;
const UNKNOWN: &str = r#"{"type":"does.not.exist"}"#;

pub(super) async fn client_event_forwarding() {
    let client = ScriptedClient::new(vec![
        Some(AUDIO_APPEND.to_owned()),
        Some(AUDIO_COMMIT.to_owned()),
        Some(UNKNOWN.to_owned()),
        None,
    ]);
    let mut fixture = ClientFixture::new(Some(SESSION_UPDATE.to_owned()), client);
    let outcome = fixture.run().await;

    assert_eq!(
        outcome.expect("the client pump ends normally"),
        ForwardEnd::Ended
    );
    let frames = fixture.stream.sent();
    let keys: Vec<Option<&str>> = frames.iter().map(event_key).collect();
    assert_eq!(
        keys,
        [
            Some("sessionStart"),
            Some("promptStart"),
            Some("contentStart"),
            Some("audioInput"),
            Some("contentEnd"),
            Some("promptEnd"),
            Some("sessionEnd"),
        ]
    );

    let prompt_name = support::numbered(1);
    let audio_name = support::numbered(3);
    assert_eq!(
        payload_text(&frames[1], "promptStart", "promptName"),
        Some(prompt_name.as_str())
    );
    assert_eq!(
        payload_text(&frames[2], "contentStart", "contentName"),
        Some(audio_name.as_str())
    );
    assert_eq!(
        payload_text(&frames[2], "contentStart", "type"),
        Some("AUDIO")
    );
    assert_eq!(
        payload_text(&frames[2], "contentStart", "role"),
        Some("USER")
    );
    assert_eq!(
        payload_text(&frames[3], "audioInput", "content"),
        Some("AAAA")
    );
    assert_eq!(
        payload_text(&frames[4], "contentEnd", "contentName"),
        Some(audio_name.as_str())
    );
    assert_eq!(
        payload_text(&frames[5], "promptEnd", "promptName"),
        Some(prompt_name.as_str())
    );
    assert_eq!(frames[6], json(r#"{"event":{"sessionEnd":{}}}"#));

    let sent = fixture.client.sent_events();
    assert_eq!(sent.len(), 1);
    let expected = json(&format!(
        r#"{{"event_id":"{}","session":{{"id":"trace-bridge-1","modalities":["text","audio"],"model":"{}"}},"type":"session.updated"}}"#,
        support::numbered(4),
        support::MODEL
    ));
    assert_eq!(sent[0], expected);

    // The ack clears the replay marker only after every transformed frame reached the stream
    assert_eq!(fixture.scope.pending_session_update, Json::Null);
    assert_eq!(fixture.scope.session_committed, Json::Bool(true));
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.stream.closed());
}

fn payload_text<'a>(frame: &'a Json, event: &str, field: &str) -> Option<&'a str> {
    let Json::Object(fields) = frame else {
        return None;
    };
    let Json::Object(body) = fields.get("event")? else {
        return None;
    };
    let Json::Object(payload) = body.get(event)? else {
        return None;
    };
    match payload.get(field) {
        Some(Json::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}
