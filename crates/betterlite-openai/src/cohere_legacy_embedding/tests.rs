#![cfg(test)]

use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::error::EngineError;
use futures_util::FutureExt as _;
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::decode::{
    DecodeInput, LegacyEmbeddingResponse, decode_cohere_legacy_embedding,
    populate_embedding_response,
};
use super::error::CohereLegacyEmbeddingError;
use super::execute::{ExecuteInput, LegacyEmbeddingTransport, execute};
use super::plan::{LegacyEmbeddingPlan, PlanInput, plan, validate_environment};
use super::{
    DEFAULT_COMPLETE_URL, EmbeddingHooks, PostCall, PreCall, SYNC_CLIENT_TIMEOUT, SdkMode,
};
use crate::cohere_embedding::TextEncoding;
use crate::openai_base_config::Headers;

const MODEL: &str = "embed-english-v3.0";
const CALLER_TIMEOUT: Duration = Duration::from_secs(30);
const FLAT_REPLY: &str =
    r#"{"embeddings":[[0.5],[0.25]],"meta":{"billed_units":{"input_tokens":2,"images":0}}}"#;

#[derive(Debug, PartialEq, Eq)]
enum Hook {
    Pre {
        input: Vec<String>,
        api_key: Option<String>,
        body: Json,
        request: Option<(Headers, String)>,
    },
    Post {
        original_response: Json,
    },
}

#[derive(Default)]
struct Recorder {
    hooks: Vec<Hook>,
}

impl EmbeddingHooks for Recorder {
    fn pre_call(&mut self, call: PreCall<'_>) {
        self.hooks.push(Hook::Pre {
            input: call.input.to_vec(),
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            body: Json::Object(call.body.clone()),
            request: call
                .headers
                .zip(call.api_base)
                .map(|(headers, api_base)| (headers.clone(), api_base.to_owned())),
        });
    }

    fn post_call(&mut self, call: PostCall<'_>) {
        self.hooks.push(Hook::Post {
            original_response: call.original_response.clone(),
        });
    }
}

#[derive(Default)]
struct Counting {
    encoded: Mutex<Vec<String>>,
}

impl Counting {
    fn encoded(&self) -> Vec<String> {
        let Ok(encoded) = self.encoded.lock() else {
            panic!("the encoding fixture lock is poisoned");
        };
        encoded.clone()
    }
}

impl TextEncoding for Counting {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        let Ok(mut encoded) = self.encoded.lock() else {
            return Err(EngineError::PoolClosed);
        };
        encoded.push(text.to_owned());
        Ok(text.chars().count())
    }
}

struct Fixture {
    status: u16,
    body: Vec<u8>,
    failure: Mutex<Option<TransportError>>,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    fn answering(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            failure: Mutex::new(None),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn failing(source: TransportError) -> Self {
        Self {
            status: 0,
            body: Vec::new(),
            failure: Mutex::new(Some(source)),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn only_request(&self) -> OutboundRequest {
        let mut seen = self.seen.lock().expect("fixture lock");
        assert_eq!(seen.len(), 1, "exactly one request");
        seen.remove(0)
    }
}

impl LegacyEmbeddingTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen.lock().expect("fixture lock").push(request);
        let failure = self.failure.lock().expect("fixture lock").take();
        let outcome = failure.map_or_else(
            || {
                Ok(UpstreamResponse {
                    status: self.status,
                    headers: HeaderMap::new(),
                    body: self.body.clone(),
                })
            },
            Err,
        );
        ready(outcome)
    }
}

struct Hanging {
    started: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl Future for Hanging {
    type Output = Result<UpstreamResponse, TransportError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.started.store(true, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for Hanging {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct HangingTransport {
    started: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl LegacyEmbeddingTransport for HangingTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        Hanging {
            started: Arc::clone(&self.started),
            dropped: Arc::clone(&self.dropped),
        }
    }
}

struct Spec {
    mode: SdkMode,
    input: Vec<String>,
    optional_params: Object,
    supplied_data: Option<Object>,
    complete_api_base: Option<String>,
    api_key: Option<String>,
    headers: Headers,
    timeout: Duration,
}

impl Spec {
    fn new() -> Self {
        Self {
            mode: SdkMode::Async,
            input: texts(&["hello"]),
            optional_params: Object::new(),
            supplied_data: None,
            complete_api_base: None,
            api_key: None,
            headers: Headers::new(),
            timeout: CALLER_TIMEOUT,
        }
    }

    fn plan(&self) -> LegacyEmbeddingPlan {
        let key = self.api_key.clone().map(SecretString::from);
        plan(&PlanInput {
            mode: self.mode,
            model: MODEL,
            input: &self.input,
            optional_params: &self.optional_params,
            supplied_data: self.supplied_data.as_ref(),
            complete_api_base: self.complete_api_base.as_deref(),
            api_key: key.as_ref(),
            headers: &self.headers,
            timeout: self.timeout,
        })
    }
}

fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn decode(
    body: &[u8],
    input: &[String],
) -> (
    Recorder,
    Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError>,
) {
    let encoding = Counting::default();
    let mut hooks = Recorder::default();
    let outcome = decode_cohere_legacy_embedding(
        DecodeInput {
            body,
            model: MODEL,
            api_key: None,
            request_data: &Object::new(),
            input,
            encoding: &encoding,
        },
        &mut hooks,
    );
    (hooks, outcome)
}

async fn run_plan(
    spec: &Spec,
    fixture: &Fixture,
    encoding: &Counting,
) -> (
    Recorder,
    Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError>,
) {
    let plan = spec.plan();
    let key = spec.api_key.clone().map(SecretString::from);
    let mut hooks = Recorder::default();
    let outcome = execute(
        fixture,
        ExecuteInput {
            plan: &plan,
            input: &spec.input,
            api_key: key.as_ref(),
            encoding,
        },
        &mut hooks,
    )
    .await;
    (hooks, outcome)
}

#[test]
fn the_default_headers_use_litellms_spelling_and_values() {
    let key = SecretString::from("sk-test");
    let merged = validate_environment(&Headers::new(), Some(&key));
    assert_eq!(
        merged,
        headers(&[
            ("Authorization", "Bearer sk-test"),
            ("Request-Source", "unspecified:litellm"),
            ("accept", "application/json"),
            ("content-type", "application/json"),
        ])
    );
}

#[test]
fn signed_headers_keep_their_case_and_value() {
    let signed = headers(&[
        ("Authorization", "AWS4-HMAC-SHA256 Credential=AKIA/20260101"),
        ("Content-Type", "application/json"),
        ("Request-Source", "bedrock"),
        ("X-Amz-Date", "20260101T000000Z"),
    ]);
    let key = SecretString::from("sk-test");
    let merged = validate_environment(&signed, Some(&key));
    let mut expected = signed;
    expected.insert(String::from("accept"), String::from("application/json"));
    assert_eq!(merged, expected);
}

#[test]
fn an_existing_header_matches_in_any_case() {
    let present = headers(&[
        ("ACCEPT", "text/plain"),
        ("AUTHORIZATION", "Basic caller"),
        ("CONTENT-TYPE", "text/csv"),
        ("request-source", "caller"),
    ]);
    let key = SecretString::from("sk-test");
    assert_eq!(validate_environment(&present, Some(&key)), present);
}

#[rstest]
#[case::absent(None)]
#[case::blank(Some(""))]
fn a_missing_or_blank_key_adds_no_authorization(#[case] api_key: Option<&str>) {
    let key = api_key.map(SecretString::from);
    let merged = validate_environment(&Headers::new(), key.as_ref());
    assert!(
        !merged
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    );
}

#[rstest]
#[case::absent(None, DEFAULT_COMPLETE_URL)]
#[case::empty(Some(""), DEFAULT_COMPLETE_URL)]
#[case::signed(
    Some("https://bedrock-runtime.example/model/m/invoke"),
    "https://bedrock-runtime.example/model/m/invoke"
)]
fn the_complete_base_chooses_the_url(#[case] base: Option<&str>, #[case] expected: &str) {
    let mut spec = Spec::new();
    spec.complete_api_base = base.map(str::to_owned);
    assert_eq!(spec.plan().url, expected);
}

#[test]
fn data_is_built_from_the_model_input_and_options() {
    let mut spec = Spec::new();
    spec.input = texts(&["first", "second"]);
    spec.optional_params = object(r#"{"input_type": "search_query", "truncate": "END"}"#);
    assert_eq!(
        spec.plan().data,
        object(
            r#"{"model": "embed-english-v3.0", "texts": ["first", "second"], "input_type": "search_query", "truncate": "END"}"#
        )
    );
}

#[test]
fn supplied_data_is_used_unchanged() {
    let mut spec = Spec::new();
    spec.supplied_data = Some(object(r#"{"texts": ["only"], "model": "other"}"#));
    assert_eq!(
        spec.plan().data,
        object(r#"{"texts": ["only"], "model": "other"}"#)
    );
}

#[test]
fn an_empty_supplied_dictionary_builds_the_request() {
    let mut spec = Spec::new();
    spec.supplied_data = Some(Object::new());
    assert_eq!(
        spec.plan().data,
        object(
            r#"{"model": "embed-english-v3.0", "texts": ["hello"], "input_type": "search_document"}"#
        )
    );
}

#[rstest]
#[case::image_last(&["plain", "data:image/png;base64,YWJj"], "images", "image")]
#[case::text_last(&["data:image/png;base64,YWJj", "plain"], "texts", "search_document")]
fn the_last_input_decides_the_modality(
    #[case] input: &[&str],
    #[case] key: &str,
    #[case] input_type: &str,
) {
    let mut spec = Spec::new();
    spec.input = texts(input);
    let data = spec.plan().data;
    assert_eq!(
        data.get("input_type"),
        Some(&Json::Text(input_type.to_owned()))
    );
    assert_eq!(
        data.get(key),
        Some(&Json::List(
            texts(input).into_iter().map(Json::Text).collect()
        ))
    );
    assert_eq!(data.len(), 3);
}

#[test]
fn an_empty_input_builds_empty_texts() {
    let mut spec = Spec::new();
    spec.input = Vec::new();
    assert_eq!(
        spec.plan().data,
        object(r#"{"model": "embed-english-v3.0", "texts": [], "input_type": "search_document"}"#)
    );
}

#[rstest]
#[case::blocking(SdkMode::Sync, SYNC_CLIENT_TIMEOUT)]
#[case::asynchronous(SdkMode::Async, CALLER_TIMEOUT)]
fn the_mode_selects_the_timeout(#[case] mode: SdkMode, #[case] expected: Duration) {
    let mut spec = Spec::new();
    spec.mode = mode;
    assert_eq!(spec.plan().timeout, expected);
}

#[tokio::test]
async fn the_async_pre_call_carries_the_body_headers_and_api_base() {
    let spec = Spec::new();
    let fixture = Fixture::answering(200, FLAT_REPLY);
    let encoding = Counting::default();
    let plan = spec.plan();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    outcome.unwrap();
    assert_eq!(
        hooks.hooks.first(),
        Some(&Hook::Pre {
            input: texts(&["hello"]),
            api_key: None,
            body: Json::Object(plan.data.clone()),
            request: Some((plan.headers.clone(), plan.url.clone())),
        })
    );
}

#[tokio::test]
async fn the_blocking_pre_call_carries_only_the_body() {
    let mut spec = Spec::new();
    spec.mode = SdkMode::Sync;
    let fixture = Fixture::answering(200, FLAT_REPLY);
    let encoding = Counting::default();
    let plan = spec.plan();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    outcome.unwrap();
    assert!(matches!(
        hooks.hooks.first(),
        Some(Hook::Pre { body, request: None, .. }) if *body == Json::Object(plan.data)
    ));
}

#[tokio::test]
async fn the_request_carries_the_plan_url_headers_and_json_body() {
    let mut spec = Spec::new();
    spec.api_key = Some(String::from("sk-test"));
    spec.headers = headers(&[("X-Amz-Date", "20260101T000000Z")]);
    let fixture = Fixture::answering(200, FLAT_REPLY);
    let encoding = Counting::default();
    let plan = spec.plan();
    let (_hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    outcome.unwrap();
    let request = fixture.only_request();
    assert_eq!(request.method, http::Method::POST);
    assert_eq!(request.url.as_str(), DEFAULT_COMPLETE_URL);
    assert_eq!(request.deadline, CALLER_TIMEOUT);
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-test")
    );
    assert_eq!(
        request
            .headers
            .get("x-amz-date")
            .and_then(|value| value.to_str().ok()),
        Some("20260101T000000Z")
    );
    assert_eq!(
        request
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let RequestBody::Json(body) = request.body else {
        panic!("the legacy handler sends a JSON body");
    };
    let sent: Json = serde_json::from_slice(&body).unwrap();
    assert_eq!(sent, Json::Object(plan.data));
}

#[tokio::test]
async fn a_status_failure_logs_the_reply_text_before_it_returns() {
    let spec = Spec::new();
    let fixture = Fixture::answering(500, r#"{"message":"upstream refused"}"#);
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    let Err(CohereLegacyEmbeddingError::Status(error)) = outcome else {
        panic!("a 500 reply must become the status error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, r#"{"message":"upstream refused"}"#);
    let [
        Hook::Pre { .. },
        Hook::Post {
            original_response, ..
        },
    ] = hooks.hooks.as_slice()
    else {
        panic!("the failure logs one pre_call and one post_call");
    };
    assert_eq!(
        *original_response,
        Json::Text(String::from(r#"{"message":"upstream refused"}"#))
    );
}

#[tokio::test]
async fn a_failure_body_that_is_not_utf8_is_logged_as_empty_text() {
    let spec = Spec::new();
    let mut fixture = Fixture::answering(502, "");
    fixture.body = vec![0xff, 0xfe];
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    let Err(CohereLegacyEmbeddingError::Status(error)) = outcome else {
        panic!("a 502 reply must become the status error");
    };
    assert_eq!(error.status_code, 502);
    assert_eq!(error.message, "");
    let [
        Hook::Pre { .. },
        Hook::Post {
            original_response, ..
        },
    ] = hooks.hooks.as_slice()
    else {
        panic!("the failure logs one pre_call and one post_call");
    };
    assert_eq!(*original_response, Json::Text(String::new()));
}

#[tokio::test]
async fn a_transport_failure_logs_the_error_before_it_returns_it() {
    let spec = Spec::new();
    let fixture = Fixture::failing(TransportError::Closed);
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    let Err(CohereLegacyEmbeddingError::Transport(source)) = outcome else {
        panic!("a transport failure must be returned unchanged");
    };
    assert!(matches!(source, TransportError::Closed));
    let [
        Hook::Pre { .. },
        Hook::Post {
            original_response, ..
        },
    ] = hooks.hooks.as_slice()
    else {
        panic!("the failure logs one pre_call and one post_call");
    };
    assert_eq!(
        *original_response,
        Json::Text(String::from(
            "the client is shutting down and takes no new requests"
        ))
    );
}

#[tokio::test]
async fn a_blocking_failure_logs_no_post_call() {
    let mut spec = Spec::new();
    spec.mode = SdkMode::Sync;
    let fixture = Fixture::answering(500, r#"{"message":"upstream refused"}"#);
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    assert!(matches!(
        outcome,
        Err(CohereLegacyEmbeddingError::Status(_))
    ));
    assert!(matches!(hooks.hooks.as_slice(), [Hook::Pre { .. }]));
}

#[tokio::test]
async fn an_unbuildable_endpoint_logs_a_post_call_in_async_mode() {
    let mut spec = Spec::new();
    spec.complete_api_base = Some(String::from("not a url"));
    let fixture = Fixture::answering(200, FLAT_REPLY);
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    assert!(matches!(outcome, Err(CohereLegacyEmbeddingError::Url(_))));
    assert!(matches!(
        hooks.hooks.as_slice(),
        [Hook::Pre { .. }, Hook::Post { .. }]
    ));
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[rstest]
#[case::informational(199, false)]
#[case::ok(200, true)]
#[case::last_success(299, true)]
#[case::redirect(300, false)]
#[case::client_error(400, false)]
#[tokio::test]
async fn only_a_two_hundred_series_status_is_a_success(
    #[case] status: u16,
    #[case] accepted: bool,
) {
    let spec = Spec::new();
    let fixture = Fixture::answering(status, FLAT_REPLY);
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    assert_eq!(outcome.is_ok(), accepted);
    let failure_logged = matches!(
        hooks.hooks.as_slice(),
        [Hook::Pre { .. }, Hook::Post { original_response, .. }]
            if *original_response == Json::Text(String::from(FLAT_REPLY))
    );
    assert_eq!(failure_logged, !accepted);
}

#[tokio::test]
async fn a_redirect_without_a_location_is_a_status_failure() {
    let spec = Spec::new();
    let fixture = Fixture::answering(307, "");
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    assert!(matches!(
        outcome,
        Err(CohereLegacyEmbeddingError::Status(error)) if error.status_code == 307
    ));
    assert!(matches!(
        hooks.hooks.as_slice(),
        [Hook::Pre { .. }, Hook::Post { .. }]
    ));
}

#[tokio::test]
async fn the_success_post_call_carries_the_parsed_reply() {
    let spec = Spec::new();
    let fixture = Fixture::answering(200, FLAT_REPLY);
    let encoding = Counting::default();
    let (hooks, outcome) = run_plan(&spec, &fixture, &encoding).await;
    let response = outcome.unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.model, MODEL);
    assert_eq!(response.data.len(), 2);
    let [
        Hook::Pre { .. },
        Hook::Post {
            original_response, ..
        },
    ] = hooks.hooks.as_slice()
    else {
        panic!("a success logs one pre_call and one post_call");
    };
    assert_eq!(*original_response, json(FLAT_REPLY));
}

#[tokio::test]
async fn dropping_the_call_cancels_the_request_without_a_post_call() {
    let transport = HangingTransport {
        started: Arc::new(AtomicBool::new(false)),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let spec = Spec::new();
    let plan = spec.plan();
    let encoding = Counting::default();
    let mut hooks = Recorder::default();
    let mut attempt = Box::pin(execute(
        &transport,
        ExecuteInput {
            plan: &plan,
            input: &spec.input,
            api_key: None,
            encoding: &encoding,
        },
        &mut hooks,
    ));
    assert!(attempt.as_mut().now_or_never().is_none());
    assert!(transport.started.load(Ordering::SeqCst));
    assert!(!transport.dropped.load(Ordering::SeqCst));
    drop(attempt);
    assert!(transport.dropped.load(Ordering::SeqCst));
    assert!(matches!(hooks.hooks.as_slice(), [Hook::Pre { .. }]));
}

#[test]
fn flat_rows_number_from_zero_without_a_type() {
    let (hooks, outcome) = decode(
        br#"{"embeddings":[[0.5],[0.25],[0.125]]}"#,
        &texts(&["hello"]),
    );
    let response = outcome.unwrap();
    assert_eq!(
        response.data,
        vec![
            super::decode::LegacyEmbeddingRecord {
                object: String::from("embedding"),
                index: 0,
                embedding: json("[0.5]"),
                kind: None,
            },
            super::decode::LegacyEmbeddingRecord {
                object: String::from("embedding"),
                index: 1,
                embedding: json("[0.25]"),
                kind: None,
            },
            super::decode::LegacyEmbeddingRecord {
                object: String::from("embedding"),
                index: 2,
                embedding: json("[0.125]"),
                kind: None,
            },
        ]
    );
    assert_eq!(hooks.hooks.len(), 1);
}

#[test]
fn grouped_rows_restart_their_index_and_carry_their_type() {
    let (_hooks, outcome) = decode(
        br#"{"embeddings":{"float":[[0.5],[0.25]],"int8":[[1],[2],[3]]}}"#,
        &texts(&["hello"]),
    );
    let response = outcome.unwrap();
    let rows: Vec<(usize, Option<&str>)> = response
        .data
        .iter()
        .map(|record| (record.index, record.kind.as_deref()))
        .collect();
    assert_eq!(
        rows,
        [
            (0, Some("float")),
            (1, Some("float")),
            (0, Some("int8")),
            (1, Some("int8")),
            (2, Some("int8"))
        ]
    );
}

#[test]
fn a_grouped_dictionary_wins_over_a_flat_response_type() {
    let (_hooks, outcome) = decode(
        br#"{"embeddings":{"float":[[0.5]]},"response_type":"float"}"#,
        &texts(&["hello"]),
    );
    let response = outcome.unwrap();
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data.first().and_then(|row| row.kind.as_deref()),
        Some("float")
    );
}

#[test]
fn a_grouped_response_type_over_flat_rows_is_rejected_after_the_post_call() {
    let (hooks, outcome) = decode(
        br#"{"embeddings":[[0.5]],"response_type":"embeddings_by_type"}"#,
        &texts(&["hello"]),
    );
    assert!(matches!(
        outcome,
        Err(CohereLegacyEmbeddingError::Embeddings)
    ));
    assert_eq!(hooks.hooks.len(), 1);
}

#[test]
fn a_malformed_reply_emits_no_success_post_call() {
    let (hooks, outcome) = decode(br#"{"embeddings":[["#, &texts(&["hello"]));
    assert!(matches!(outcome, Err(CohereLegacyEmbeddingError::Json(_))));
    assert!(hooks.hooks.is_empty());
}

#[test]
fn a_reply_without_embeddings_logs_the_post_call_then_fails() {
    let (hooks, outcome) = decode(br#"{"meta":{}}"#, &texts(&["hello"]));
    assert!(matches!(
        outcome,
        Err(CohereLegacyEmbeddingError::Embeddings)
    ));
    assert_eq!(hooks.hooks.len(), 1);
}

#[rstest]
#[case::root_array(b"[]")]
#[case::embeddings_text(br#"{"embeddings":"rows"}"#)]
#[case::group_not_a_list(br#"{"embeddings":{"float":1}}"#)]
fn a_misshapen_reply_is_rejected(#[case] body: &[u8]) {
    let (hooks, outcome) = decode(body, &texts(&["hello"]));
    assert!(matches!(
        outcome,
        Err(CohereLegacyEmbeddingError::Embeddings)
    ));
    assert_eq!(hooks.hooks.len(), 1);
}

#[test]
fn populate_embedding_response_fills_a_decoded_record_on_its_own() {
    let reply = json(r#"{"embeddings":[[0.5]]}"#);
    let encoding = Counting::default();
    let (_hooks, decoded) = decode(br#"{"embeddings":[[0.5]]}"#, &texts(&["hello"]));
    let populated =
        populate_embedding_response(&reply, MODEL, &texts(&["hello"]), &encoding).unwrap();
    assert_eq!(populated, decoded.unwrap());
}

#[test]
fn billed_units_skip_the_tokenizer() {
    let (hooks, outcome) = decode(FLAT_REPLY.as_bytes(), &texts(&["hello", "world"]));
    let response = outcome.unwrap();
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.total_tokens, 2);
    let billed = response.usage.prompt_tokens_details.unwrap();
    assert_eq!(
        (billed.text_tokens, billed.image_tokens),
        (Some(2), Some(0))
    );
    assert_eq!(hooks.hooks.len(), 1);
}

#[test]
fn an_unbilled_reply_counts_every_input_once() {
    let encoding = Counting::default();
    let reply = json(r#"{"embeddings":[[0.5]]}"#);
    let response =
        populate_embedding_response(&reply, MODEL, &texts(&["hello", "worlds"]), &encoding)
            .unwrap();
    assert_eq!(response.usage.prompt_tokens, 11);
    assert!(response.usage.prompt_tokens_details.is_none());
    assert_eq!(encoding.encoded(), texts(&["hello", "worlds"]));
}

#[rstest]
fn replays_the_cohere_legacy_embedding_corpus(
    #[files("fuzz/corpus/cohere_legacy_embedding/*")] path: PathBuf,
) {
    let body = fs::read(path).unwrap();
    let (_hooks, outcome) = decode(&body, &texts(&["hello"]));
    let _unused = outcome;
}
