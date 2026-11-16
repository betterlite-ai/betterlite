use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE};

use super::support::{
    Clock, Outcome, RecordedBody, Script, TIMEOUT, audio, auth, options, prepared, remote,
};
use crate::api::soniox::prepare::Prepared;
use crate::api::soniox::prepare::error::ReplyError;
use crate::api::soniox::prepare::options::HandlerOptions;
use crate::api::soniox::remote::error::RemoteError;

fn uploaded_filename(transport: &Script) -> Option<String> {
    let calls = transport.calls();
    let RecordedBody::Multipart { parts, .. } = &calls[0].body else {
        panic!("the upload must send a multipart body");
    };
    parts[0].file_name.clone()
}

fn armed(options: HandlerOptions) -> (Script, Clock, Prepared) {
    let transport = Script::new(vec![Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"id":"file-1"}"#,
    }]);
    (transport, Clock::default(), prepared(options, &auth()))
}

#[tokio::test]
async fn upload_sends_only_the_authorization_header() {
    let (transport, sleeper, ready) = armed(options());
    let soniox = remote(&transport, &sleeper, &ready);
    let id = soniox.upload(&audio()).await.unwrap();
    assert_eq!(id, "file-1");
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.method, Method::POST);
    assert_eq!(call.url, "https://api.soniox.test/v1/files");
    assert_eq!(call.headers.len(), 1);
    assert_eq!(
        call.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer sk-local"
    );
    assert!(call.headers.get(CONTENT_TYPE).is_none());
    assert_eq!(call.deadline, TIMEOUT);
    let RecordedBody::Multipart { boundary, parts } = &call.body else {
        panic!("the upload must send a multipart body");
    };
    assert_eq!(boundary, "boundary-7");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].name, "file");
    assert_eq!(parts[0].file_name.as_deref(), Some("audio.mp3"));
    assert_eq!(parts[0].content_type.as_deref(), Some("audio/mpeg"));
    assert_eq!(parts[0].data, b"RIFF-data".to_vec());
}

#[tokio::test]
async fn upload_uses_the_filename_override() {
    let mut chosen = options();
    chosen.filename_override = Some("renamed.wav".to_owned());
    let (transport, sleeper, ready) = armed(chosen);
    let soniox = remote(&transport, &sleeper, &ready);
    soniox.upload(&audio()).await.unwrap();
    assert_eq!(
        uploaded_filename(&transport).as_deref(),
        Some("renamed.wav")
    );
}

#[tokio::test]
async fn upload_falls_back_when_the_override_is_absent() {
    let (transport, sleeper, ready) = armed(options());
    let soniox = remote(&transport, &sleeper, &ready);
    soniox.upload(&audio()).await.unwrap();
    assert_eq!(uploaded_filename(&transport).as_deref(), Some("audio.mp3"));
}

#[tokio::test]
async fn upload_falls_back_when_the_override_is_empty() {
    let mut chosen = options();
    chosen.filename_override = Some(String::new());
    let (transport, sleeper, ready) = armed(chosen);
    let soniox = remote(&transport, &sleeper, &ready);
    soniox.upload(&audio()).await.unwrap();
    assert_eq!(uploaded_filename(&transport).as_deref(), Some("audio.mp3"));
}

#[tokio::test]
async fn upload_reports_the_provider_error_before_the_id() {
    let transport = Script::new(vec![Outcome::Reply {
        status: 500,
        headers: vec![("x-request-id", "req-1")],
        body: r#"{"id":"file-1","error_message":"boom"}"#,
    }]);
    let sleeper = Clock::default();
    let ready = prepared(options(), &auth());
    let soniox = remote(&transport, &sleeper, &ready);
    let error = soniox.upload(&audio()).await.unwrap_err();
    let RemoteError::Provider(provider) = error else {
        panic!("a 500 must surface as the provider error");
    };
    assert_eq!(
        provider.message,
        "Soniox upload file failed (HTTP 500): boom"
    );
    assert_eq!(provider.status_code, 500);
    let headers = provider.headers.unwrap();
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    assert_eq!(transport.calls().len(), 1);
}

#[tokio::test]
async fn upload_requires_the_id_of_a_successful_reply() {
    let transport = Script::new(vec![Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"status":"ok"}"#,
    }]);
    let sleeper = Clock::default();
    let ready = prepared(options(), &auth());
    let soniox = remote(&transport, &sleeper, &ready);
    let error = soniox.upload(&audio()).await.unwrap_err();
    assert!(matches!(
        error,
        RemoteError::Reply(ReplyError::Missing { field: "id" })
    ));
}

#[tokio::test]
async fn upload_reports_the_missing_authorization_header() {
    let transport = Script::new(Vec::new());
    let sleeper = Clock::default();
    let ready = prepared(options(), &[("x-client", "suite")]);
    let soniox = remote(&transport, &sleeper, &ready);
    let error = soniox.upload(&audio()).await.unwrap_err();
    assert!(matches!(
        error,
        RemoteError::MissingHeader {
            name: "Authorization"
        }
    ));
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn upload_propagates_a_transport_failure_without_retrying() {
    let transport = Script::new(vec![
        Outcome::Fail(TransportError::Closed),
        Outcome::Reply {
            status: 200,
            headers: Vec::new(),
            body: r#"{"id":"file-1"}"#,
        },
    ]);
    let sleeper = Clock::default();
    let ready = prepared(options(), &auth());
    let soniox = remote(&transport, &sleeper, &ready);
    let error = soniox.upload(&audio()).await.unwrap_err();
    assert!(matches!(
        error,
        RemoteError::Transport(TransportError::Closed)
    ));
    assert_eq!(transport.calls().len(), 1);
}
