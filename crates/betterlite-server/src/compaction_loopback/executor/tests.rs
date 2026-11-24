#![cfg(test)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderName, HeaderValue};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::Response;
use axum::routing::post;
use betterlite_anthropic::native_compaction::protocol::{CompactionProtocol, Mapping};
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_models::decisions::Json;
use betterlite_router::compaction_executor::{
    CompactionExecutor as _, is_native_compaction_call, native_compaction_parent,
};
use indexmap::IndexMap;
use rstest::rstest;
use tokio::task::try_id;

use super::{LoopbackExecutor, MAX_CHILD_BODY_BYTES};
use crate::compaction_loopback::error::LoopbackError;

const CHILD_MODEL: &str = "child-model";
const FOUR_MEBIBYTES: usize = 4 * 1024 * 1024;
const PARENT_MODEL: &str = "parent-model";
const RECORDED_BODY_LIMIT: usize = 16 * 1024 * 1024;
const REDACTION_OPT_OUT: &str = "litellm-disable-message-redaction";

struct Seen {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    peer: Option<SocketAddr>,
    privacy: Option<PrivacyContext>,
    native_parent: Option<String>,
    native_child: bool,
    spawned_task: bool,
}

struct Recorder {
    status: StatusCode,
    body: Vec<u8>,
    seen: Mutex<Vec<Seen>>,
}

impl Recorder {
    const fn new(status: StatusCode, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            seen: Mutex::new(Vec::new()),
        }
    }
}

async fn record(State(recorder): State<Arc<Recorder>>, request: Request<Body>) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|inside| inside.0);
    let privacy = request.extensions().get::<PrivacyContext>().copied();
    let native_child = is_native_compaction_call();
    let native_parent = native_compaction_parent(CHILD_MODEL);
    let spawned_task = try_id().is_some();
    let body = to_bytes(request.into_body(), RECORDED_BODY_LIMIT)
        .await
        .expect("the recorded request body is readable");
    recorder.seen.lock().unwrap().push(Seen {
        method,
        uri,
        headers,
        body,
        peer,
        privacy,
        native_parent,
        native_child,
        spawned_task,
    });
    Response::builder()
        .status(recorder.status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(recorder.body.clone()))
        .expect("the recorded reply is a valid response")
}

fn router(recorder: Arc<Recorder>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(record))
        .route("/v1/messages", post(record))
        .with_state(recorder)
}

fn summary_payload() -> Mapping {
    IndexMap::from([
        ("model".to_owned(), Json::Text(CHILD_MODEL.to_owned())),
        (
            "messages".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([(
                "role".to_owned(),
                Json::Text("user".to_owned()),
            )]))]),
        ),
    ])
}

fn executor_with(
    recorder: Arc<Recorder>,
    parent_uri: &'static str,
    headers: HeaderMap,
    peer: Option<SocketAddr>,
) -> LoopbackExecutor {
    LoopbackExecutor {
        app: router(recorder),
        parent_uri: Uri::from_static(parent_uri),
        root_path: String::new(),
        headers,
        peer,
    }
}

fn executor_of(status: StatusCode, body: Vec<u8>) -> (LoopbackExecutor, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::new(status, body));
    let executor = executor_with(
        Arc::clone(&recorder),
        "/v1/chat/completions",
        HeaderMap::new(),
        None,
    );
    (executor, recorder)
}

async fn summarize(
    executor: &LoopbackExecutor,
    protocol: CompactionProtocol,
    payload: &Mapping,
    logging_disabled: bool,
) -> Mapping {
    executor
        .execute(protocol, payload, Some(PARENT_MODEL), logging_disabled)
        .await
        .unwrap()
}

fn take_child(recorder: &Recorder) -> Seen {
    let mut seen = recorder.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    seen.pop().expect("the child request was recorded")
}

#[rstest]
#[case(CompactionProtocol::Chat, "/v1/chat/completions")]
#[case(CompactionProtocol::Messages, "/v1/messages")]
#[tokio::test]
async fn the_child_reaches_the_protocol_route_with_the_parent_request(
    #[case] protocol: CompactionProtocol,
    #[case] expected_path: &'static str,
) {
    let peer = "203.0.113.7:4321".parse::<SocketAddr>().unwrap();
    let recorder = Arc::new(Recorder::new(
        StatusCode::OK,
        br#"{"marker":"child"}"#.to_vec(),
    ));
    let executor = executor_with(
        Arc::clone(&recorder),
        "/v1/chat/completions?stream=true",
        HeaderMap::new(),
        Some(peer),
    );
    let request_payload = summary_payload();
    let result = summarize(&executor, protocol, &request_payload, false).await;
    assert_eq!(result.get("marker"), Some(&Json::Text("child".to_owned())));
    let child = take_child(&recorder);
    assert_eq!(child.method, Method::POST);
    assert_eq!(child.uri.path(), expected_path);
    assert_eq!(child.uri.query(), None);
    assert_eq!(child.peer, Some(peer));
}

#[tokio::test]
async fn the_child_carries_the_parent_headers_and_the_payload_as_json() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-child"));
    headers.insert(
        HeaderName::from_static("content-length"),
        HeaderValue::from_static("999"),
    );
    let recorder = Arc::new(Recorder::new(StatusCode::OK, b"{}".to_vec()));
    let executor = executor_with(Arc::clone(&recorder), "/v1/chat/completions", headers, None);
    let request_payload = summary_payload();
    summarize(&executor, CompactionProtocol::Chat, &request_payload, false).await;
    let child = take_child(&recorder);
    assert_eq!(child.headers.get(AUTHORIZATION).unwrap(), "Bearer sk-child");
    assert_eq!(child.headers.get(CONTENT_TYPE).unwrap(), "application/json");
    let body_length = child.body.len().to_string();
    assert_eq!(
        child.headers.get(CONTENT_LENGTH).unwrap(),
        body_length.as_str()
    );
    assert_eq!(
        serde_json::from_slice::<Mapping>(&child.body).unwrap(),
        request_payload,
    );
}

#[tokio::test]
async fn a_call_runs_in_its_own_task_with_the_compaction_scope_and_privacy_flag() {
    let recorder = Arc::new(Recorder::new(
        StatusCode::OK,
        br#"{"marker":"child"}"#.to_vec(),
    ));
    let executor = executor_with(
        Arc::clone(&recorder),
        "/v1/chat/completions",
        HeaderMap::new(),
        None,
    );
    let request_payload = summary_payload();
    summarize(&executor, CompactionProtocol::Chat, &request_payload, true).await;
    assert!(!is_native_compaction_call());
    let child = take_child(&recorder);
    assert!(child.spawned_task);
    assert!(child.native_child);
    assert_eq!(child.native_parent.as_deref(), Some(PARENT_MODEL));
    assert_eq!(
        child.privacy,
        Some(PrivacyContext {
            message_logging_disabled: true,
        })
    );
}

#[tokio::test]
async fn a_call_without_a_parent_model_sets_only_the_child_flag() {
    let (executor, recorder) = executor_of(StatusCode::OK, b"{}".to_vec());
    let request_payload = summary_payload();
    let result = executor
        .execute(CompactionProtocol::Chat, &request_payload, None, false)
        .await
        .unwrap();
    assert_eq!(result, Mapping::new());
    let child = take_child(&recorder);
    assert!(child.native_child);
    assert_eq!(child.native_parent, None);
}

#[tokio::test]
async fn a_call_without_a_peer_address_leaves_the_connect_info_off() {
    let (executor, recorder) = executor_of(StatusCode::OK, b"{}".to_vec());
    let request_payload = summary_payload();
    summarize(&executor, CompactionProtocol::Chat, &request_payload, false).await;
    assert_eq!(take_child(&recorder).peer, None);
}

#[tokio::test]
async fn two_calls_on_one_executor_use_their_own_logging_flag() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("litellm-disable-message-redaction"),
        HeaderValue::from_static("true"),
    );
    let recorder = Arc::new(Recorder::new(StatusCode::OK, b"{}".to_vec()));
    let executor = executor_with(Arc::clone(&recorder), "/v1/chat/completions", headers, None);
    let request_payload = summary_payload();
    summarize(&executor, CompactionProtocol::Chat, &request_payload, true).await;
    summarize(&executor, CompactionProtocol::Chat, &request_payload, false).await;
    let seen = recorder.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(!seen[0].headers.contains_key(REDACTION_OPT_OUT));
    assert!(seen[1].headers.contains_key(REDACTION_OPT_OUT));
    assert_eq!(
        seen[0].privacy,
        Some(PrivacyContext {
            message_logging_disabled: true,
        })
    );
    assert_eq!(seen[1].privacy, Some(PrivacyContext::default()));
}

#[rstest]
#[case("{}")]
#[case("{\"model\":7}")]
#[case("{\"model\":null}")]
#[case("{\"model\":[\"x\"]}")]
#[case("{\"model\":{\"name\":\"x\"}}")]
#[tokio::test]
async fn a_missing_or_non_string_model_answers_missing_model_without_a_request(
    #[case] payload_json: &'static str,
) {
    let request_payload = serde_json::from_str::<Mapping>(payload_json).unwrap();
    let (executor, recorder) = executor_of(StatusCode::OK, b"{}".to_vec());
    let error = executor
        .execute(
            CompactionProtocol::Chat,
            &request_payload,
            Some(PARENT_MODEL),
            false,
        )
        .await
        .expect_err("a payload without a string model has no child call");
    assert_eq!(
        error.0.downcast_ref::<LoopbackError>(),
        Some(&LoopbackError::MissingModel)
    );
    assert_eq!(
        error.to_string(),
        "the native compaction payload has no string model name"
    );
    assert!(recorder.seen.lock().unwrap().is_empty());
}

#[rstest]
#[case(300_u16)]
#[case(400_u16)]
#[case(500_u16)]
#[tokio::test]
async fn a_non_success_status_answers_child_failed_with_the_status(#[case] status_code: u16) {
    let status = StatusCode::from_u16(status_code).unwrap();
    let (executor, _recorder) = executor_of(status, b"{}".to_vec());
    let request_payload = summary_payload();
    let error = executor
        .execute(
            CompactionProtocol::Chat,
            &request_payload,
            Some(PARENT_MODEL),
            false,
        )
        .await
        .expect_err("a non-success child is an error");
    assert_eq!(
        error.0.downcast_ref::<LoopbackError>(),
        Some(&LoopbackError::ChildFailed {
            status: status_code,
        })
    );
    assert_eq!(
        error.to_string(),
        format!("Native compaction child request failed (HTTP {status_code})")
    );
}

#[rstest]
#[case("[]")]
#[case("42")]
#[case("\"text\"")]
#[case("null")]
#[case("not json")]
#[case("{\"a\":1} trailing")]
#[tokio::test]
async fn a_body_that_is_not_a_json_object_answers_invalid_child_body(
    #[case] child_body: &'static str,
) {
    let (executor, _recorder) = executor_of(StatusCode::OK, child_body.as_bytes().to_vec());
    let request_payload = summary_payload();
    let error = executor
        .execute(
            CompactionProtocol::Chat,
            &request_payload,
            Some(PARENT_MODEL),
            false,
        )
        .await
        .expect_err("a body that is not a JSON object is an error");
    assert_eq!(
        error.0.downcast_ref::<LoopbackError>(),
        Some(&LoopbackError::InvalidChildBody)
    );
    assert_eq!(
        error.to_string(),
        "Native compaction child returned an invalid JSON object"
    );
}

#[rstest]
#[case(200_u16)]
#[case(299_u16)]
#[tokio::test]
async fn a_successful_child_returns_the_parsed_object(#[case] status_code: u16) {
    let status = StatusCode::from_u16(status_code).unwrap();
    let (executor, _recorder) = executor_of(
        status,
        br#"{"marker":"child","usage":{"cached":true}}"#.to_vec(),
    );
    let request_payload = summary_payload();
    let result = summarize(&executor, CompactionProtocol::Chat, &request_payload, false).await;
    let expected = IndexMap::from([
        ("marker".to_owned(), Json::Text("child".to_owned())),
        (
            "usage".to_owned(),
            Json::Object(IndexMap::from([("cached".to_owned(), Json::Bool(true))])),
        ),
    ]);
    assert_eq!(result, expected);
}

#[tokio::test]
async fn a_child_body_at_the_limit_is_read_and_parsed() {
    let mut body = b"{\"blob\":\"".to_vec();
    body.resize(MAX_CHILD_BODY_BYTES - 2, b'a');
    body.extend_from_slice(b"\"}");
    assert_eq!(body.len(), FOUR_MEBIBYTES);
    let (executor, _recorder) = executor_of(StatusCode::OK, body);
    let request_payload = summary_payload();
    let result = summarize(&executor, CompactionProtocol::Chat, &request_payload, false).await;
    assert!(result.contains_key("blob"));
}

#[tokio::test]
async fn a_child_body_over_the_limit_is_an_error() {
    let body = vec![b' '; MAX_CHILD_BODY_BYTES + 1];
    let (executor, _recorder) = executor_of(StatusCode::OK, body);
    let request_payload = summary_payload();
    let error = executor
        .execute(
            CompactionProtocol::Chat,
            &request_payload,
            Some(PARENT_MODEL),
            false,
        )
        .await
        .expect_err("a child body over the limit is an error");
    assert_eq!(error.to_string(), "length limit exceeded");
}
