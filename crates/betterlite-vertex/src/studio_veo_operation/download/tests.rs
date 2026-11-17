#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use tokio::time::timeout;

use super::{ContentLookup, DownloadTransport, content_request, download_content, download_target};
use crate::studio_veo_operation::error::StudioVeoOperationError;

const OPERATION: &str = "operations/generate_1234567890";
const BASE: &str = "https://generativelanguage.googleapis.com/";
const STATUS_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/operations/generate_1234567890";
const VIDEO: &str = "https://generativelanguage.googleapis.com/v1beta/files/abc:download?alt=media";
const COMPLETED: &[u8] = br#"{"name":"operations/generate_1234567890","done":true,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"https://generativelanguage.googleapis.com/v1beta/files/abc:download?alt=media"}},{"video":{"uri":"https://example.test/second.mp4"}}]}}}"#;
const NOT_DONE: &[u8] = br#"{"name":"operations/generate_1234567890","done":false,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"https://example.test/video.mp4"}}]}}}"#;
const NO_RESPONSE: &[u8] = br#"{"name":"operations/generate_1234567890","done":true}"#;
const NO_SAMPLES: &[u8] = br#"{"name":"operations/generate_1234567890","done":true,"response":{"generateVideoResponse":{"generatedSamples":[]}}}"#;
const RELATIVE_TARGET: &[u8] = br#"{"name":"operations/generate_1234567890","done":true,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"files/abc:download"}}]}}}"#;

#[derive(Clone)]
enum Reply {
    Status { status: u16, body: Vec<u8> },
    Failure,
}

impl Reply {
    fn answer(self) -> Result<UpstreamResponse, TransportError> {
        match self {
            Self::Status { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Self::Failure => Err(TransportError::Closed),
        }
    }
}

#[derive(Clone)]
struct Recorded {
    method: Method,
    url: String,
    query: Vec<(String, String)>,
    headers: HeaderMap,
    deadline: Duration,
    empty_body: bool,
}

struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

impl Scripted {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    fn record(&self, request: OutboundRequest) -> Reply {
        self.requests.lock().unwrap().push(Recorded {
            method: request.method,
            url: request.url.to_string(),
            query: request.query,
            headers: request.headers,
            deadline: request.deadline,
            empty_body: matches!(request.body, RequestBody::Empty),
        });
        self.replies.lock().unwrap().pop_front().unwrap()
    }
}

impl DownloadTransport for Scripted {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.record(request).answer())
    }
}

// a transport whose answer never arrives, so the test can observe the caller's drop closing it
struct Stalled {
    dropped: Arc<AtomicBool>,
}

impl Stalled {
    fn new() -> Self {
        Self {
            dropped: Arc::default(),
        }
    }
}

struct Silent {
    dropped: Arc<AtomicBool>,
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl Future for Silent {
    type Output = Result<UpstreamResponse, TransportError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl DownloadTransport for Stalled {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        Silent {
            dropped: Arc::clone(&self.dropped),
        }
    }
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-goog-api-key"),
        HeaderValue::from_static("test-key"),
    );
    headers
}

fn lookup(headers: &HeaderMap) -> ContentLookup<'_> {
    ContentLookup {
        video_id: OPERATION,
        api_base: BASE,
        headers,
        deadline: Duration::from_secs(30),
    }
}

#[tokio::test]
async fn lookup_then_download() {
    let transport = Scripted::new(vec![
        Reply::Status {
            status: 200,
            body: COMPLETED.to_vec(),
        },
        Reply::Status {
            status: 200,
            body: b"video-bytes".to_vec(),
        },
    ]);
    let headers = headers();
    let request = content_request(&lookup(&headers), &transport)
        .await
        .expect("the status lookup resolves");
    let content = download_content(request, &transport)
        .await
        .expect("the artifact download returns");
    assert_eq!(content, b"video-bytes");
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url, STATUS_URL);
    assert_eq!(requests[1].method, Method::GET);
    assert_eq!(requests[1].url, VIDEO);
    for request in &requests {
        assert_eq!(
            request.headers.get("x-goog-api-key"),
            Some(&HeaderValue::from_static("test-key"))
        );
        assert!(request.query.is_empty());
        assert!(request.empty_body);
        assert_eq!(request.deadline, Duration::from_secs(30));
    }
}

#[rstest]
#[case::redirection(302)]
#[case::client_error(400)]
#[case::server_error(500)]
#[tokio::test]
async fn status_failure_stops_download(#[case] status: u16) {
    let transport = Scripted::new(vec![Reply::Status {
        status,
        body: br#"{"error":{"code":400,"message":"bad"}}"#.to_vec(),
    }]);
    let headers = headers();
    let outcome = content_request(&lookup(&headers), &transport).await;
    let Err(StudioVeoOperationError::Provider { url, response }) = outcome else {
        panic!("a {status} status stops the lookup");
    };
    assert_eq!(url, STATUS_URL);
    assert_eq!(response.status, status);
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn an_invalid_status_url_is_rejected() {
    let transport = Scripted::new(vec![Reply::Failure]);
    let headers = headers();
    let outcome = content_request(
        &ContentLookup {
            video_id: OPERATION,
            api_base: "not a url",
            headers: &headers,
            deadline: Duration::from_secs(30),
        },
        &transport,
    )
    .await;
    assert!(matches!(outcome, Err(StudioVeoOperationError::Url { .. })));
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn a_transport_failure_stops_the_lookup() {
    let transport = Scripted::new(vec![Reply::Failure]);
    let headers = headers();
    let outcome = content_request(&lookup(&headers), &transport).await;
    assert!(matches!(
        outcome,
        Err(StudioVeoOperationError::Transport { .. })
    ));
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn an_unfinished_operation_has_no_target() {
    assert!(matches!(
        download_target(NOT_DONE),
        Err(StudioVeoOperationError::NotComplete)
    ));
}

#[test]
fn a_completed_operation_without_a_response_has_no_target() {
    assert!(matches!(
        download_target(NO_RESPONSE),
        Err(StudioVeoOperationError::NoResponse)
    ));
}

#[test]
fn a_completed_operation_without_samples_has_no_target() {
    assert!(matches!(
        download_target(NO_SAMPLES),
        Err(StudioVeoOperationError::NoSamples)
    ));
}

#[test]
fn a_malformed_operation_has_no_target() {
    assert!(matches!(
        download_target(b"not json"),
        Err(StudioVeoOperationError::Json(_))
    ));
}

#[test]
fn the_first_generated_sample_is_the_target() {
    let target = download_target(COMPLETED).expect("the completed operation names its video");
    assert_eq!(target, VIDEO);
}

#[tokio::test]
async fn an_invalid_artifact_uri_is_rejected() {
    let transport = Scripted::new(vec![Reply::Status {
        status: 200,
        body: RELATIVE_TARGET.to_vec(),
    }]);
    let headers = headers();
    let outcome = content_request(&lookup(&headers), &transport).await;
    assert!(matches!(outcome, Err(StudioVeoOperationError::Url { .. })));
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn a_failed_download_still_returns_its_bytes() {
    let transport = Scripted::new(vec![
        Reply::Status {
            status: 200,
            body: COMPLETED.to_vec(),
        },
        Reply::Status {
            status: 404,
            body: b"denied".to_vec(),
        },
    ]);
    let headers = headers();
    let request = content_request(&lookup(&headers), &transport)
        .await
        .expect("the status lookup resolves");
    let content = download_content(request, &transport)
        .await
        .expect("the download transform copies the bytes whatever status carried them");
    assert_eq!(content, b"denied");
    let requests = transport.requests();
    assert_eq!(requests[1].url, VIDEO);
}

#[tokio::test]
async fn cancel_closes_status() {
    let transport = Stalled::new();
    let headers = headers();
    let outcome = timeout(
        Duration::from_millis(100),
        content_request(&lookup(&headers), &transport),
    )
    .await;
    assert!(outcome.is_err());
    assert!(transport.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_closes_download() {
    let transport = Stalled::new();
    let status = Scripted::new(vec![Reply::Status {
        status: 200,
        body: COMPLETED.to_vec(),
    }]);
    let headers = headers();
    let request = content_request(&lookup(&headers), &status)
        .await
        .expect("the status lookup resolves");
    let outcome = timeout(
        Duration::from_millis(100),
        download_content(request, &transport),
    )
    .await;
    assert!(outcome.is_err());
    assert!(transport.dropped.load(Ordering::SeqCst));
}
