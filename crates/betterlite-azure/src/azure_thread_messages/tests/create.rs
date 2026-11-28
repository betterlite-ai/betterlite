#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::threads::error::{ReplyFault, ThreadsError};
use betterlite_openai::threads::records::{ThreadCreate, ThreadRecord};

use crate::azure_thread_messages::error::ThreadMessagesError;
use crate::azure_thread_messages::operations::ThreadOperation;

use super::support::{
    Auth, ERROR_BODY, Fixture, KEY, PLAIN_THREAD, THREAD, THREADS_URL, failing_harness, harness,
    status_of, wire,
};

// Both fields present in the caller's data reach the SDK body; neither gets a default.
const CREATE_BODY: &str = r#"{"messages":[{"role":"user","content":"hello"},{"role":"assistant","content":[{"type":"text","text":"hi"}]}],"metadata":{"team":"a"}}"#;
const EMPTY_CONTAINERS: &str = r#"{"messages":[],"metadata":{}}"#;
const BAD_OBJECT: &str = r#"{"id":"thread_1","object":"banana","created_at":7,"metadata":null}"#;
const EXTRA_FIELDS: &str = r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":null,"tool_resources":{"code_interpreter":{"file_ids":["file-1"]}}}"#;

pub(super) async fn check() {
    create_bodies().await;
    extra_fields_dropped().await;
    invalid_object().await;
    invalid_shape().await;
    status_failure().await;
    malformed_reply().await;
    transport_failure().await;
}

fn data(text: &str) -> ThreadCreate {
    serde_json::from_str(text).unwrap()
}

// The caller's own messages and metadata reach the SDK body untouched, including an explicit [] or {}.
async fn create_bodies() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for (body, reply, flag, kind) in [
        ("{}", PLAIN_THREAD, None, ClientKind::Sync),
        (CREATE_BODY, THREAD, Some(false), ClientKind::Sync),
        (EMPTY_CONTAINERS, THREAD, None, ClientKind::Sync),
        (CREATE_BODY, THREAD, Some(true), ClientKind::Asynchronous),
    ] {
        let create = data(body);
        let (api, transport) = harness(200, reply);
        let created = api
            .call(&request, None, flag, ThreadOperation::Create(&create))
            .await
            .unwrap();
        assert_eq!(created.kind, kind, "{body}");
        assert_eq!(
            created.thread,
            serde_json::from_str::<ThreadRecord>(reply).unwrap(),
            "{body}"
        );
        assert_eq!(
            transport.requests(),
            vec![wire("POST", THREADS_URL, Some(body), Auth::Key(KEY))],
            "{body}"
        );
    }
}

// The whole reply goes through the shared Thread record, which keeps its fields and drops unknown ones.
async fn extra_fields_dropped() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create = data("{}");
    let (api, _transport) = harness(200, EXTRA_FIELDS);
    let created = api
        .call(&request, None, None, ThreadOperation::Create(&create))
        .await
        .unwrap();
    assert_eq!(
        created.thread,
        serde_json::from_str::<ThreadRecord>(PLAIN_THREAD).unwrap()
    );
}

// LiteLLM's Thread declares object: Literal["thread"], so another value is a validation failure.
async fn invalid_object() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create = data("{}");
    let (api, transport) = harness(200, BAD_OBJECT);
    let failed = api
        .call(&request, None, None, ThreadOperation::Create(&create))
        .await;
    assert!(
        matches!(
            failed,
            Err(ThreadMessagesError::Threads(ThreadsError::Reply {
                fault: ReplyFault::Shape,
                ..
            }))
        ),
        "{failed:?}"
    );
    assert_eq!(transport.requests().len(), 1);
}

async fn invalid_shape() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create = data("{}");
    for body in [
        r#"{"id":"thread_1"}"#,
        r#"{"id":5,"created_at":7,"object":"thread"}"#,
    ] {
        let (api, transport) = harness(200, body);
        let failed = api
            .call(&request, None, None, ThreadOperation::Create(&create))
            .await;
        assert!(
            matches!(
                failed,
                Err(ThreadMessagesError::Threads(ThreadsError::Reply {
                    fault: ReplyFault::Shape,
                    ..
                }))
            ),
            "{body}: {failed:?}"
        );
        assert_eq!(transport.requests().len(), 1, "{body}");
    }
}

// A status failure wins over a body that happens to look like a thread, and the reply body rides on the error.
async fn status_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create = data("{}");
    for (status, body) in [(400, THREAD), (429, ERROR_BODY), (500, ERROR_BODY)] {
        let (api, transport) = harness(status, body);
        let failed = api
            .call(&request, None, None, ThreadOperation::Create(&create))
            .await
            .unwrap_err();
        let (seen, seen_body) = status_of(failed);
        assert_eq!(seen, status);
        assert_eq!(seen_body, body.as_bytes(), "{status}");
        assert_eq!(transport.requests().len(), 1, "{status}");
    }
}

async fn malformed_reply() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create = data("{}");
    for body in ["", "not json"] {
        let (api, _transport) = harness(200, body);
        let failed = api
            .call(&request, None, None, ThreadOperation::Create(&create))
            .await;
        assert!(
            matches!(
                failed,
                Err(ThreadMessagesError::Threads(ThreadsError::Reply {
                    fault: ReplyFault::Malformed | ReplyFault::Truncated,
                    ..
                }))
            ),
            "{body:?}: {failed:?}"
        );
    }
}

async fn transport_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create = data("{}");
    let (api, transport) = failing_harness();
    let failed = api
        .call(&request, None, None, ThreadOperation::Create(&create))
        .await;
    assert!(
        matches!(
            failed,
            Err(ThreadMessagesError::Threads(ThreadsError::Transport { .. }))
        ),
        "{failed:?}"
    );
    assert_eq!(transport.requests().len(), 1);
}
