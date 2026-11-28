#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::threads::error::{ReplyFault, ThreadsError};
use betterlite_openai::threads::records::ThreadRecord;

use crate::azure_thread_messages::error::ThreadMessagesError;
use crate::azure_thread_messages::operations::ThreadOperation;

use super::support::{
    Auth, ERROR_BODY, Fixture, KEY, THREAD, THREAD_URL, failing_harness, harness, status_of, wire,
};

const QUOTED_ID: &str = "a/b c!$&'()*+,;=:@-._~\u{e9}%";
const QUOTED_URL: &str = "https://example.test/openai/threads/a%2Fb%20c!$&'()*+,;=:@-._~%C3%A9%25?api-version=2024-02-01";

pub(super) async fn check() {
    plain_retrieve().await;
    quoted_identifier().await;
    async_retrieve().await;
    empty_identifier().await;
    dot_segments().await;
    status_failure().await;
    invalid_shape().await;
    transport_failure().await;
}

async fn plain_retrieve() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    let retrieved = api
        .call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(retrieved.kind, ClientKind::Sync);
    assert_eq!(
        retrieved.thread,
        serde_json::from_str::<ThreadRecord>(THREAD).unwrap()
    );
    assert_eq!(
        transport.requests(),
        vec![wire("GET", THREAD_URL, None, Auth::Key(KEY))]
    );
}

// The SDK quotes the id as one path parameter, leaving the RFC 3986 sub-delimiters, ':' and '@' bare.
async fn quoted_identifier() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve(QUOTED_ID))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", QUOTED_URL, None, Auth::Key(KEY))]
    );
}

async fn async_retrieve() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    let retrieved = api
        .call(
            &request,
            None,
            Some(true),
            ThreadOperation::Retrieve("thread_1"),
        )
        .await
        .unwrap();
    assert_eq!(retrieved.kind, ClientKind::Asynchronous);
    assert_eq!(
        transport.requests(),
        vec![wire("GET", THREAD_URL, None, Auth::Key(KEY))]
    );
}

// An empty id is refused before any request is built, as the SDK's path template does.
async fn empty_identifier() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    let failed = api
        .call(&request, None, None, ThreadOperation::Retrieve(""))
        .await;
    assert!(
        matches!(
            failed,
            Err(ThreadMessagesError::Threads(ThreadsError::EmptyThreadId))
        ),
        "{failed:?}"
    );
    assert!(transport.requests().is_empty());
}

async fn dot_segments() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for segment in [".", ".."] {
        let (api, transport) = harness(200, THREAD);
        let failed = api
            .call(&request, None, None, ThreadOperation::Retrieve(segment))
            .await;
        assert!(
            matches!(
                failed,
                Err(ThreadMessagesError::Threads(
                    ThreadsError::DotSegment { .. }
                ))
            ),
            "{segment}: {failed:?}"
        );
        assert!(transport.requests().is_empty(), "{segment}");
    }
}

// The rejected status and its reply body reach the caller unchanged.
async fn status_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for status in [400, 404, 500] {
        let (api, transport) = harness(status, ERROR_BODY);
        let failed = api
            .call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
            .await
            .unwrap_err();
        let (seen, seen_body) = status_of(failed);
        assert_eq!(seen, status);
        assert_eq!(seen_body, ERROR_BODY.as_bytes(), "{status}");
        assert_eq!(transport.requests().len(), 1, "{status}");
    }
}

async fn invalid_shape() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, r#"{"created_at":7,"object":"thread"}"#);
    let failed = api
        .call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
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

async fn transport_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = failing_harness();
    let failed = api
        .call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
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
