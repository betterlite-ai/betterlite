use betterlite_models::decisions::Json;
use uuid::Uuid;

use super::support::{self, SECRET};
use crate::realtime_events::ack::AckError;
use crate::realtime_events::decode::json_dict;

pub(super) async fn ack_session_update_order() {
    stream_failure_first().await;
    no_trace().await;
    with_trace().await;
    modalities_from_message().await;
    modalities_default().await;
    invalid_modalities().await;
    send_failure().await;
}

async fn stream_failure_first() {
    let mut setup = support::AckSetup::new();
    setup.scope.pending_session_update = Json::Text("update".to_owned());
    setup.stream.error = Some("boom".to_owned());
    let result = setup.run().await;
    assert!(matches!(result, Err(AckError::Stream(_))));
    assert_eq!(setup.stream.awaited(), 1);
    assert_eq!(setup.scope.session_committed, Json::Null);
    assert_eq!(
        setup.scope.pending_session_update,
        Json::Text("update".to_owned())
    );
    assert!(setup.client.sent().is_empty());
}

async fn no_trace() {
    let mut setup = support::AckSetup::new();
    setup.scope.pending_session_update = Json::Text("update".to_owned());
    setup.message = support::preface(r#"{"session":{"modalities":3}}"#);
    assert!(setup.run().await.expect("acked"));
    assert_eq!(setup.stream.awaited(), 1);
    assert_eq!(setup.scope.session_committed, Json::Bool(true));
    assert_eq!(setup.scope.pending_session_update, Json::Null);
    assert!(setup.client.sent().is_empty());
}

async fn with_trace() {
    let mut setup = support::AckSetup::new();
    setup.trace_id = Some("trace-7".to_owned());
    setup.message = support::preface(r#"{"session":{"modalities":["text","audio"]}}"#);
    assert!(setup.run().await.expect("acked"));
    let sent = setup.client.sent();
    assert_eq!(sent.len(), 1);
    let expected = support::json(&format!(
        r#"{{"event_id":"{}","session":{{"id":"trace-7","modalities":["text","audio"],"model":"amazon.nova-sonic-v1:0"}},"type":"session.updated"}}"#,
        Uuid::from_u128(1)
    ));
    assert_eq!(support::json(&sent[0]), expected);
    assert_eq!(setup.scope.session_committed, Json::Bool(true));
    assert_eq!(setup.scope.pending_session_update, Json::Null);
}

async fn acked_modalities(message: &str) -> Json {
    let mut setup = support::AckSetup::new();
    setup.trace_id = Some("trace".to_owned());
    setup.message = support::preface(message);
    assert!(setup.run().await.expect("acked"));
    let sent = setup.client.sent();
    let object = support::json(&sent[0]);
    json_dict(&object)
        .and_then(|fields| fields.get("session"))
        .and_then(json_dict)
        .and_then(|session| session.get("modalities"))
        .cloned()
        .expect("the modalities of the ack")
}

async fn modalities_from_message() {
    let audio = acked_modalities(r#"{"session":{"modalities":["audio"]}}"#).await;
    assert_eq!(audio, support::json(r#"["audio"]"#));
    let empty = acked_modalities(r#"{"session":{"modalities":[]}}"#).await;
    assert_eq!(empty, support::json("[]"));
}

async fn modalities_default() {
    let null = acked_modalities(r#"{"session":{"modalities":null}}"#).await;
    assert_eq!(null, support::json(r#"["text","audio"]"#));
    let missing = acked_modalities(r#"{"session":{}}"#).await;
    assert_eq!(missing, support::json(r#"["text","audio"]"#));
    let no_session = acked_modalities(r#"{"type":"session.update"}"#).await;
    assert_eq!(no_session, support::json(r#"["text","audio"]"#));
}

async fn invalid_modalities() {
    let messages = [
        r#"{"session":{"modalities":{"a":1}}}"#,
        r#"{"session":{"modalities":[1]}}"#,
        r#"{"session":{"modalities":"text"}}"#,
    ];
    for message in messages {
        let mut setup = support::AckSetup::new();
        setup.trace_id = Some("trace".to_owned());
        setup.message = support::preface(message);
        let result = setup.run().await;
        assert!(matches!(result, Err(AckError::Modalities)));
        assert_eq!(setup.stream.awaited(), 1);
        assert_eq!(setup.scope.session_committed, Json::Bool(true));
        assert_eq!(setup.scope.pending_session_update, Json::Null);
        assert!(setup.client.sent().is_empty());
    }
}

async fn send_failure() {
    let (log, _guard) = support::capture();
    let mut setup = support::AckSetup::new();
    setup.trace_id = Some("trace".to_owned());
    setup.client.send_error = Some(format!("socket {SECRET} closed"));
    assert!(!setup.run().await.expect("forwarding ended"));
    assert_eq!(setup.client.sent().len(), 1);
    assert_eq!(setup.scope.session_committed, Json::Bool(true));
    assert!(
        log.text()
            .contains("Client to Bedrock forwarding ended: socket REDACTED closed")
    );
    assert!(!log.text().contains(SECRET));
}
