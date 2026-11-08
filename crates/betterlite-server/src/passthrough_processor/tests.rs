#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::Response;
use betterlite_config::sections::settings::JsonValue;
use indexmap::IndexMap;

use super::{
    BaseProcessCall, ExceptionMapper, LlmPassthroughProcessor, LlmRequestProcessor,
    PassthroughCall, PassthroughProcessor as _, ProcessedReply, UpstreamReply,
};

#[derive(Debug)]
struct PortError(String);

#[derive(Clone, Copy)]
enum ReplyKind {
    Streaming,
    Direct,
    Upstream,
    Fail,
}

struct RecordedCall {
    route_type: String,
    model: Option<String>,
    record: IndexMap<String, JsonValue>,
    method: String,
    response_headers: HeaderMap,
    key: String,
    router: Option<String>,
}

struct FakeProcessor {
    kind: ReplyKind,
    seen: Mutex<Option<RecordedCall>>,
}

impl FakeProcessor {
    fn new(kind: ReplyKind) -> Self {
        Self {
            kind,
            seen: Mutex::new(None),
        }
    }

    fn seen(&self) -> Option<RecordedCall> {
        self.seen.lock().expect("record lock").take()
    }
}

#[async_trait]
impl LlmRequestProcessor<String, String> for FakeProcessor {
    type Error = PortError;

    async fn base_process_llm_request(
        &self,
        call: BaseProcessCall<'_, String, String>,
    ) -> Result<ProcessedReply, PortError> {
        let recorded = RecordedCall {
            route_type: call.route_type.to_owned(),
            model: call.model.map(str::to_owned),
            record: call.record.clone(),
            method: call.request.method.as_str().to_owned(),
            response_headers: call.response_headers.clone(),
            key: call.key.clone(),
            router: call.router.cloned(),
        };
        *self.seen.lock().expect("record lock") = Some(recorded);
        let branch = match self.kind {
            ReplyKind::Streaming => ProcessedReply::Streaming(
                Response::builder()
                    .status(StatusCode::CREATED)
                    .header("x-branch", "streaming")
                    .body(Body::from("stream body"))
                    .expect("streaming reply"),
            ),
            ReplyKind::Direct => ProcessedReply::Direct(
                Response::builder()
                    .status(StatusCode::ACCEPTED)
                    .header("x-branch", "direct")
                    .body(Body::from("direct body"))
                    .expect("direct reply"),
            ),
            ReplyKind::Upstream => ProcessedReply::Upstream(UpstreamReply {
                status: StatusCode::BAD_GATEWAY,
                headers: upstream_headers(),
                body: Bytes::from_static(b"upstream body"),
            }),
            ReplyKind::Fail => return Err(PortError(String::from("boom"))),
        };
        Ok(branch)
    }
}

fn upstream_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("13"));
    headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
    headers.insert("server", HeaderValue::from_static("upstream"));
    headers.insert("date", HeaderValue::from_static("today"));
    headers.insert("connection", HeaderValue::from_static("close"));
    headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
    headers.insert("content-encoding", HeaderValue::from_static("gzip"));
    headers.insert("x-upstream", HeaderValue::from_static("yes"));
    headers.insert("x-shared", HeaderValue::from_static("upstream"));
    headers
}

struct FakeMapper;

#[async_trait]
impl ExceptionMapper<String, PortError> for FakeMapper {
    type Mapped = String;

    async fn handle_llm_api_exception(&self, error: PortError, key: &String) -> String {
        format!("mapped {} for {key}", error.0)
    }
}

fn request_parts(method: &str, uri: &str) -> Parts {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(())
        .expect("request")
        .into_parts()
        .0
}

fn call(response_headers: HeaderMap) -> PassthroughCall<String, String> {
    let mut record = IndexMap::new();
    record.insert(
        String::from("model"),
        JsonValue::String(String::from("gpt-4o")),
    );
    record.insert(String::from("stream"), JsonValue::Bool(true));
    PassthroughCall {
        record,
        request: request_parts("POST", "/v1/chat/completions"),
        response_headers,
        key: String::from("key-1"),
        router: Some(String::from("router-1")),
        model: Some(String::from("gpt-4o")),
    }
}

fn processor(kind: ReplyKind) -> LlmPassthroughProcessor<FakeProcessor, FakeMapper> {
    LlmPassthroughProcessor {
        processor: FakeProcessor::new(kind),
        exceptions: FakeMapper,
    }
}

async fn body_text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    String::from_utf8(bytes.to_vec()).expect("utf-8 body")
}

#[rstest::rstest]
#[case::streaming(ReplyKind::Streaming, StatusCode::CREATED, "stream body")]
#[case::direct(ReplyKind::Direct, StatusCode::ACCEPTED, "direct body")]
#[tokio::test]
async fn streaming_and_direct_replies_are_returned_as_they_are(
    #[case] kind: ReplyKind,
    #[case] status: StatusCode,
    #[case] body: &'static str,
) {
    let response = processor(kind)
        .process_passthrough(call(HeaderMap::new()))
        .await
        .expect("reply");
    assert_eq!(response.status(), status);
    let branch = if matches!(kind, ReplyKind::Streaming) {
        "streaming"
    } else {
        "direct"
    };
    assert_eq!(
        response.headers().get("x-branch"),
        Some(&HeaderValue::from_static(branch))
    );
    assert_eq!(body_text(response).await, body);
}

#[tokio::test]
async fn the_processor_sends_the_route_type_and_the_call_fields_to_the_port() {
    let mut response_headers = HeaderMap::new();
    response_headers.insert("x-route", HeaderValue::from_static("owned"));
    let mut passthrough = call(response_headers);
    passthrough
        .record
        .insert(String::from("temperature"), JsonValue::from(1_i32));
    let processor = processor(ReplyKind::Streaming);
    processor
        .process_passthrough(passthrough)
        .await
        .expect("reply");
    let seen = processor.processor.seen().expect("recorded call");
    assert_eq!(seen.route_type, "allm_passthrough_route");
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.model.as_deref(), Some("gpt-4o"));
    assert_eq!(seen.key, "key-1");
    assert_eq!(seen.router.as_deref(), Some("router-1"));
    assert_eq!(
        seen.response_headers.get("x-route"),
        Some(&HeaderValue::from_static("owned"))
    );
    let keys: Vec<&str> = seen.record.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["model", "stream", "temperature"]);
    assert_eq!(seen.record.get("stream"), Some(&JsonValue::Bool(true)));
}

#[tokio::test]
async fn an_upstream_reply_becomes_a_response_with_filtered_headers() {
    let mut custom = HeaderMap::new();
    custom.insert("content-type", HeaderValue::from_static("application/json"));
    custom.insert("content-length", HeaderValue::from_static("99"));
    custom.insert("x-shared", HeaderValue::from_static("route"));
    custom.insert("x-custom", HeaderValue::from_static("kept"));
    let response = processor(ReplyKind::Upstream)
        .process_passthrough(call(custom))
        .await
        .expect("reply");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let headers = response.headers();
    assert_eq!(
        headers.get("x-upstream"),
        Some(&HeaderValue::from_static("yes"))
    );
    assert_eq!(
        headers.get("x-custom"),
        Some(&HeaderValue::from_static("kept"))
    );
    assert_eq!(
        headers.get("x-shared"),
        Some(&HeaderValue::from_static("route"))
    );
    assert_eq!(
        headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert_excluded_headers_are_absent(headers);
    assert_eq!(body_text(response).await, "upstream body");
}

fn assert_excluded_headers_are_absent(headers: &HeaderMap) {
    for excluded in [
        "content-length",
        "transfer-encoding",
        "content-encoding",
        "server",
        "date",
        "connection",
        "keep-alive",
    ] {
        assert!(headers.get(excluded).is_none(), "{excluded} survived");
    }
}

#[tokio::test]
async fn a_port_failure_goes_through_the_exception_mapper() {
    let result = processor(ReplyKind::Fail)
        .process_passthrough(call(HeaderMap::new()))
        .await;
    assert_eq!(result.expect_err("mapped failure"), "mapped boom for key-1");
}
