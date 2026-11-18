use std::error::Error;
use std::io;

use betterlite_models::decisions::Json;

use super::support::{self, SECRET};
use super::transport;
use crate::realtime_events::client::{ClientError, ClientMessages};
use crate::realtime_events::decode::decode_realtime_client_preface;
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::StreamError;

pub(super) async fn session_update_first() {
    client_messages().await;
    native_errors();
    preface_decoding();
}

async fn client_messages() {
    let (log, _guard) = support::capture();
    let client = transport::FakeClient::new();
    {
        let mut received = client.received.lock().unwrap();
        received.push_back(transport::Received::Text("first".to_owned()));
        received.push_back(transport::Received::Error(format!("{SECRET} died")));
    }
    let redactor = support::redactor();
    let mut messages: ClientMessages<'_, io::Error> = ClientMessages {
        client: &client,
        initial: Some(String::new()),
        redactor: &redactor,
        done: false,
    };
    assert_eq!(messages.next().await.as_deref(), Some(""));
    assert_eq!(messages.next().await.as_deref(), Some("first"));
    assert_eq!(messages.next().await, None);
    client
        .received
        .lock()
        .unwrap()
        .push_back(transport::Received::Text("later".to_owned()));
    assert_eq!(messages.next().await, None);
    assert_eq!(client.received.lock().unwrap().len(), 1);
    assert!(
        log.text()
            .contains("Client to Bedrock forwarding ended: REDACTED died")
    );
    assert!(!log.text().contains(SECRET));
    let fresh = transport::FakeClient::new();
    let mut empty: ClientMessages<'_, io::Error> = ClientMessages {
        client: &fresh,
        initial: None,
        redactor: &redactor,
        done: false,
    };
    assert_eq!(empty.next().await, None);
}

// The protocol errors keep the transport error they came from as their cause
fn native_errors() {
    let client: ClientError<io::Error> = ClientError {
        message: "gone".to_owned(),
        source: Some(io::Error::other("socket closed")),
    };
    assert_eq!(
        client.source().expect("a connector cause").to_string(),
        "socket closed"
    );
    let stream: StreamError<io::Error> = StreamError {
        message: "failed".to_owned(),
        source: Some(io::Error::other("tls")),
    };
    assert_eq!(
        stream.source().expect("a transport cause").to_string(),
        "tls"
    );
}

fn preface_decoding() {
    assert!(support::preface("[]").is_empty());
    assert!(support::preface("{").is_empty());
    assert!(support::preface("3").is_empty());
    assert!(decode_realtime_client_preface(&[0xff]).is_empty());
    assert_eq!(support::preface(r#"{"a":{"b":1}}"#).len(), 1);
    let parsed = support::preface(r#"{"session":{"modalities":["text"]}}"#);
    assert_eq!(
        parsed.get("session"),
        Some(&support::json(r#"{"modalities":["text"]}"#))
    );
}

pub(super) fn pending_session_update() {
    committed_errors();
    identity_only();
    pending_reads();
}

fn committed_errors() {
    let mut scope = EventScope::default();
    scope.session_committed = Json::Bool(true);
    scope.committed_failure = Json::Text("boom".to_owned());
    let error = scope.pending_session_update().expect_err("committed");
    assert_eq!(error.status_code, 400);
    assert_eq!(
        error.message,
        "Bedrock realtime session already committed to a provider stream; it cannot be replayed. The committed stream failed with: boom"
    );
    scope.committed_failure = Json::Text(String::new());
    assert_eq!(
        scope
            .pending_session_update()
            .expect_err("committed")
            .message,
        "Bedrock realtime session already committed to a provider stream; it cannot be replayed"
    );
    scope.committed_failure = Json::Null;
    assert!(scope.pending_session_update().is_err());
    scope.committed_failure = support::json(r#"{"boom":1}"#);
    assert!(scope.pending_session_update().is_err());
}

fn identity_only() {
    let mut scope = EventScope::default();
    scope.pending_session_update = Json::Text("update".to_owned());
    let not_committed = [
        Json::Bool(false),
        support::json("1"),
        Json::Text("true".to_owned()),
        Json::Null,
    ];
    for committed in not_committed {
        scope.session_committed = committed;
        assert_eq!(
            scope
                .pending_session_update()
                .expect("not committed")
                .as_deref(),
            Some("update")
        );
    }
    scope.session_committed = support::json("1");
    assert!(scope.pending_session_update().is_ok());
}

fn pending_reads() {
    let mut scope = EventScope::default();
    assert_eq!(scope.pending_session_update().expect("not committed"), None);
    scope.pending_session_update = Json::Text(String::new());
    assert_eq!(
        scope
            .pending_session_update()
            .expect("not committed")
            .as_deref(),
        Some("")
    );
    scope.pending_session_update = Json::Bool(false);
    assert_eq!(scope.pending_session_update().expect("not committed"), None);
    scope.pending_session_update = support::json("5");
    assert_eq!(scope.pending_session_update().expect("not committed"), None);
    scope.pending_session_update = support::json(r#"["a"]"#);
    assert_eq!(scope.pending_session_update().expect("not committed"), None);
    scope.commit();
    assert_eq!(scope.pending_session_update, Json::Null);
    assert_eq!(scope.session_committed, Json::Bool(true));
    assert!(scope.pending_session_update().is_err());
}
