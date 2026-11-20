#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::files::request::create_file_parts;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::{MultipartPart, encode};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogLevel, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{
    FileContentProvider, ProviderUploadPayload, ProviderUploadValue, TwoStepFileUploadConfig,
    TwoStepFileUploadRequest, UploadData, UploadUrlLocation,
};
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use futures_util::StreamExt as _;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use tokio_util::task::TaskTracker;

use crate::api::api_boundaries::ApiLimits;
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError};

use super::config::{FileParams, FileResponse, FileUploadConfig, Headers, UploadParams};
use super::error::{TransformFailure, UploadError, UrlFailure};
use super::plan::{
    MultipartPlan, PresignedPlan, TwoStepPlan, UploadPlan, bytes_json, initial_data_json,
};

mod decoder;
mod media;
use super::service::{
    ClientSource, LogContext, Mode, UploadBody, UploadCall, UploadDeps, UploadRequest,
    UploadTransport, prepare, send,
};

const BOUNDARY: &str = "b2461-boundary";
const API_BASE: &str = "http://api.example.test/v1/files";
const UPLOAD_URL: &str = "http://uploads.example.test/gcs/upload-1";

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    lookup(value, name).expect("the field is logged")
}

fn lookup<'json>(value: &'json Json, name: &str) -> Option<&'json Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        other => panic!("expected an object, found {other:?}"),
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn status(value: u16) -> Json {
    Json::Number(Number::from(value))
}

fn number(value: usize) -> Json {
    Json::Number(Number::from(value))
}

fn answer(status: u16, headers: &[(&str, &str)], body: &[u8]) -> UpstreamResponse {
    let headers = headers
        .iter()
        .map(|(name, value)| {
            (
                name.parse().expect("a header name"),
                HeaderValue::from_str(value).expect("a header value"),
            )
        })
        .collect();
    UpstreamResponse {
        status,
        headers,
        body: body.to_vec(),
    }
}

#[derive(Debug)]
struct Seen {
    method: Method,
    url: String,
    headers: HeaderMap,
    timeout: Option<Duration>,
    body: Vec<Vec<u8>>,
}

#[derive(Clone, Default)]
struct Wire {
    seen: Arc<Mutex<Vec<Seen>>>,
    answers: Arc<Mutex<VecDeque<UpstreamResponse>>>,
}

impl Wire {
    fn with(answers: Vec<UpstreamResponse>) -> Self {
        Self {
            seen: Arc::default(),
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .expect("the wire is not poisoned")
            .drain(..)
            .collect()
    }
}

#[derive(Clone, Default)]
struct FakeTransport {
    wire: Wire,
}

impl UploadTransport for FakeTransport {
    async fn send(&self, request: UploadRequest) -> Result<UpstreamResponse, TransportError> {
        let UploadRequest {
            method,
            url,
            headers,
            body,
            timeout,
        } = request;
        let body = match body {
            UploadBody::Bytes(data) => vec![data],
            UploadBody::Multipart { boundary, parts } => {
                vec![encode(&boundary, &parts).expect("a multipart body")]
            }
            UploadBody::Blocks(mut blocks) => {
                let mut chunks = Vec::new();
                while let Some(chunk) = blocks.next().await {
                    chunks.push(chunk);
                }
                chunks
            }
        };
        self.wire
            .seen
            .lock()
            .expect("the wire is not poisoned")
            .push(Seen {
                method,
                url: url.to_string(),
                headers,
                timeout,
                body,
            });
        let mut answers = self.wire.answers.lock().expect("the wire is not poisoned");
        answers.pop_front().ok_or(TransportError::Closed)
    }
}

struct FakeSource {
    transport: FakeTransport,
}

impl ClientSource for FakeSource {
    type Client = FakeTransport;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<FakeTransport, ClientError> {
        Ok(self.transport.clone())
    }
}

struct Config {
    calls: Mutex<Vec<&'static str>>,
    plan: Mutex<Option<UploadPlan>>,
    base: Option<String>,
    method: Method,
}

impl Config {
    fn new(plan: UploadPlan, base: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            plan: Mutex::new(Some(plan)),
            base: Some(base.to_owned()),
            method: Method::POST,
        }
    }

    fn with_method(plan: UploadPlan, base: &str, method: Method) -> Self {
        Self {
            method,
            ..Self::new(plan, base)
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .expect("the config is not poisoned")
            .clone()
    }
}

impl FileUploadConfig for Config {
    fn validate_environment(&self, headers: Headers, _params: &UploadParams<'_>) -> Headers {
        self.calls
            .lock()
            .expect("the config is not poisoned")
            .push("validate_environment");
        headers
    }

    fn get_complete_file_url(
        &self,
        _params: &UploadParams<'_>,
        _data: &FileCreate,
    ) -> Result<Option<String>, UrlFailure> {
        self.calls
            .lock()
            .expect("the config is not poisoned")
            .push("get_complete_file_url");
        Ok(self.base.clone())
    }

    fn file_upload_http_method(&self) -> Method {
        self.method
    }

    fn transform_create_file_request(
        &self,
        _data: &FileCreate,
        _params: &UploadParams<'_>,
    ) -> Result<UploadPlan, TransformFailure> {
        self.calls
            .lock()
            .expect("the config is not poisoned")
            .push("transform_create_file_request");
        let plan = self
            .plan
            .lock()
            .expect("the config is not poisoned")
            .take()
            .expect("one transform per call");
        Ok(plan)
    }

    fn transform_create_file_response(
        &self,
        response: &FileResponse,
        params: &FileParams,
    ) -> Result<Json, BaseLlmException> {
        self.calls
            .lock()
            .expect("the config is not poisoned")
            .push("transform_create_file_response");
        let headers = response
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        Ok(Json::Object(IndexMap::from([
            ("status".to_owned(), status(response.status_code)),
            (
                "url".to_owned(),
                response
                    .url
                    .as_deref()
                    .map_or(Json::Null, |url| Json::Text(url.to_owned())),
            ),
            ("body".to_owned(), bytes_json(&response.body)),
            ("headers".to_owned(), Json::Object(headers)),
            ("params".to_owned(), Json::Object(params.clone())),
        ])))
    }
}

#[derive(Default)]
struct RecordingSink {
    records: Mutex<Vec<LogRecord>>,
}

impl LogSink for RecordingSink {
    fn emit(&self, record: LogRecord) {
        self.records
            .lock()
            .expect("the sink is not poisoned")
            .push(record);
    }
}

impl RecordingSink {
    fn messages(&self, level: LogLevel) -> Vec<String> {
        self.records
            .lock()
            .expect("the sink is not poisoned")
            .iter()
            .filter(|record| record.level == level)
            .map(|record| record.message.clone())
            .collect()
    }
}

struct Harness {
    config: Config,
    source: FakeSource,
    tasks: TaskTracker,
    state: LoggingState,
    redactor: Redactor,
    sink: RecordingSink,
}

impl Harness {
    fn new(plan: UploadPlan, base: &str, answers: Vec<UpstreamResponse>) -> Self {
        Self::build(Config::new(plan, base), answers)
    }

    fn build(config: Config, answers: Vec<UpstreamResponse>) -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"files","call_type":"create_file","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("a logging init");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor");
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("a fixed now");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default(),
        )
        .expect("a logging state");
        Self {
            config,
            source: FakeSource {
                transport: FakeTransport {
                    wire: Wire::with(answers),
                },
            },
            tasks: TaskTracker::new(),
            state,
            redactor,
            sink: RecordingSink::default(),
        }
    }

    async fn run(
        &mut self,
        mode: Mode,
        call: &FileCreate,
        params: &FileParams,
    ) -> Result<Json, UploadError> {
        let upload_params = UploadParams {
            api_base: Some(API_BASE),
            api_key: None,
            ssl_verify: None,
            litellm_params: params,
        };
        let upload_call = UploadCall {
            data: call,
            params: &upload_params,
        };
        let boundaries = || String::from(BOUNDARY);
        let Self {
            config,
            source,
            tasks,
            state,
            redactor,
            sink,
        } = self;
        let deps = UploadDeps {
            config,
            source,
            supplied: None,
            tasks,
            boundaries: &boundaries,
            limits: ApiLimits::default(),
        };
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor,
            log: sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("a fixed now"),
        };
        let mut log = LogContext { state, env: &env };
        let prepared = prepare(&deps, mode, &upload_call, &mut log)?;
        send(prepared).await
    }

    fn seen(&self) -> Vec<Seen> {
        self.source.transport.wire.seen()
    }

    fn details(&self) -> Json {
        Json::Object(self.state.model_call_details.clone())
    }
}

fn params() -> FileParams {
    IndexMap::from([(String::from("api_key"), text("sk-test"))])
}

fn create_call() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some(String::from("rows.jsonl")),
            content_type: Some(String::from("application/jsonl")),
            data: b"{\"row\":1}\n".to_vec(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn two_step_plan(config: TwoStepFileUploadConfig) -> UploadPlan {
    UploadPlan::TwoStep(Box::new(TwoStepPlan {
        initial_method: Some(config.initial_request.method),
        initial_url: Some(config.initial_request.url),
        initial_headers: config.initial_request.headers,
        initial_data: initial_data_json(&config.initial_request.data),
        upload_method: Some(config.upload_request.method),
        upload_url: Some(config.upload_request.url),
        upload_headers: config.upload_request.headers,
        upload_data: config.upload_request.data,
        url_location: Some(config.upload_url_location),
        url_key: config.upload_url_key,
    }))
}

fn manus_plan() -> UploadPlan {
    two_step_plan(manus_config())
}

fn manus_config() -> TwoStepFileUploadConfig {
    TwoStepFileUploadConfig {
        initial_request: request(
            "POST",
            API_BASE,
            BTreeMap::from([(String::from("API_KEY"), String::from("sk-manus"))]),
            UploadData::Provider(ProviderUploadPayload {
                provider: FileContentProvider::Manus,
                value: ProviderUploadValue {
                    filename: String::from("rows.jsonl"),
                },
            }),
        ),
        upload_request: request(
            "PUT",
            "",
            BTreeMap::new(),
            UploadData::Bytes(b"{\"row\":1}\n".to_vec()),
        ),
        upload_url_location: UploadUrlLocation::Body,
        upload_url_key: Some(String::from("upload_url")),
    }
}

fn request(
    method: &str,
    url: &str,
    headers: BTreeMap<String, String>,
    data: UploadData,
) -> TwoStepFileUploadRequest {
    TwoStepFileUploadRequest {
        method: String::from(method),
        url: String::from(url),
        headers,
        data,
    }
}

fn text_plan() -> UploadPlan {
    UploadPlan::Text(String::from("{\"display_name\":\"rows.jsonl\"}"))
}

#[tokio::test]
async fn upload_plan_order() {
    let mut harness = Harness::new(text_plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    let result = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(
        harness.config.calls(),
        [
            "validate_environment",
            "get_complete_file_url",
            "transform_create_file_request",
            "transform_create_file_response",
        ]
    );
    let seen = harness.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(seen[0].url, API_BASE);
    assert_eq!(
        seen[0].body,
        vec![b"{\"display_name\":\"rows.jsonl\"}".to_vec()]
    );
    assert_eq!(field(&result, "status"), &status(200));
    // the sync handler hands the transform a copy carrying api_base as upload_url
    assert_eq!(
        field(field(&result, "params"), "upload_url"),
        &text(API_BASE)
    );
}

#[tokio::test]
async fn upload_plan_order_missing_api_base() {
    let mut config = Config::new(text_plan(), API_BASE);
    config.base = None;
    let mut harness = Harness::build(config, Vec::new());
    let error = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "api_base is required for create_file");
    assert_eq!(
        harness.config.calls(),
        ["validate_environment", "get_complete_file_url"]
    );
    assert!(harness.seen().is_empty());
}

#[tokio::test]
async fn upload_plan_order_unsupported_plan() {
    let plan = UploadPlan::Unsupported(Box::new(Json::Object(IndexMap::new())));
    let mut harness = Harness::new(plan, API_BASE, Vec::new());
    let error = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unsupported transformed_request type: <class 'dict'>"
    );
    assert!(harness.seen().is_empty());
    assert!(
        !harness
            .config
            .calls()
            .contains(&"transform_create_file_response")
    );
}

#[tokio::test]
async fn upload_plan_order_real_anthropic_plan() {
    let file = FileData::Tuple3 {
        filename: Some(String::from("rows.jsonl")),
        content: FileBody::Bytes(b"{\"row\":1}\n".to_vec()),
        content_type: Some(String::from("application/jsonl")),
    };
    let parts: Vec<MultipartPart> =
        create_file_parts(Some(&file), Some("batch"), NOW).expect("anthropic parts");
    let plan = UploadPlan::Multipart(Box::new(MultipartPlan { parts }));
    let mut harness = Harness::new(plan, API_BASE, vec![answer(200, &[], b"{}")]);
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    let body = std::str::from_utf8(&seen[0].body[0]).expect("a utf-8 multipart body");
    assert!(body.contains("name=\"file\"; filename=\"rows.jsonl\""));
    assert!(body.contains("name=\"purpose\""));
    assert!(body.ends_with(&format!("--{BOUNDARY}--\r\n")));
    let content_type = seen[0]
        .headers
        .get(CONTENT_TYPE)
        .expect("a multipart content type");
    assert_eq!(
        content_type.to_str().expect("a header value"),
        format!("multipart/form-data; boundary={BOUNDARY}")
    );
}

#[tokio::test]
async fn two_step_upload() {
    let mut harness = Harness::new(
        manus_plan(),
        API_BASE,
        vec![
            answer(
                200,
                &[("content-type", "application/json")],
                b"{\"upload_url\":\"http://uploads.example.test/manus/1\",\"extra\":true}",
            ),
            answer(200, &[], b"{\"id\":\"file-1\"}"),
        ],
    );
    let result = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(seen[0].url, API_BASE);
    assert_eq!(
        seen[0].body,
        vec![b"{\"filename\":\"rows.jsonl\"}".to_vec()]
    );
    assert_eq!(
        seen[0].headers.get("api_key").expect("the initial key"),
        "sk-manus"
    );
    // the second request carries only upload_request headers and data
    assert_eq!(seen[1].method, Method::PUT);
    assert_eq!(seen[1].url, "http://uploads.example.test/manus/1");
    assert_eq!(seen[1].body, vec![b"{\"row\":1}\n".to_vec()]);
    assert!(seen[1].headers.is_empty());
    // the truthy response body is kept for the transform
    let saved = field(field(&result, "params"), "initial_file_response");
    assert_eq!(field(saved, "extra"), &Json::Bool(true));
}

#[tokio::test]
async fn two_step_upload_headers_location() {
    // no upload_url_location means headers, and no key is read there
    let plan = two_step_plan(TwoStepFileUploadConfig {
        initial_request: TwoStepFileUploadRequest {
            method: String::from("POST"),
            url: String::from(API_BASE),
            headers: BTreeMap::from([(
                String::from("X-Goog-Upload-Command"),
                String::from("start"),
            )]),
            data: UploadData::Text(String::from("{\"file\":{}}")),
        },
        upload_request: TwoStepFileUploadRequest {
            method: String::from("POST"),
            url: String::new(),
            headers: BTreeMap::from([(
                String::from("X-Goog-Upload-Command"),
                String::from("upload, finalize"),
            )]),
            data: UploadData::Bytes(b"payload".to_vec()),
        },
        upload_url_location: UploadUrlLocation::Headers,
        upload_url_key: Some(String::from("ignored-key")),
    });
    let mut harness = Harness::new(
        plan,
        API_BASE,
        vec![
            answer(
                200,
                &[("X-Goog-Upload-URL", UPLOAD_URL)],
                b"{\"not\":\"read\"}",
            ),
            answer(200, &[], b"{}"),
        ],
    );
    let result = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert_eq!(seen[1].url, UPLOAD_URL);
    // a header-location response keeps no body for the transform
    assert_eq!(
        lookup(field(&result, "params"), "initial_file_response"),
        None
    );
}

#[rstest]
#[case::empty_object("{}")]
#[case::null_value("{\"upload_url\":null}")]
#[case::empty_value("{\"upload_url\":\"\"}")]
#[case::number_value("{\"upload_url\":7}")]
#[case::not_json("not json")]
#[case::array("[1,2]")]
#[case::nothing("")]
#[tokio::test]
async fn two_step_upload_failure_skips_the_second_request(#[case] body: &str) {
    let mut harness = Harness::new(
        manus_plan(),
        API_BASE,
        vec![answer(
            200,
            &[("content-type", "application/json")],
            body.as_bytes(),
        )],
    );
    let error = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    match error {
        UploadError::Provider(provider) => {
            assert_eq!(provider.status_code, 500);
            assert!(provider.status_code_is_synthesized);
            assert_eq!(
                provider.message,
                "Failed to get upload URL from initial request"
            );
        }
        other => panic!("expected a provider error, found {other}"),
    }
    assert_eq!(harness.seen().len(), 1);
    assert!(
        !harness
            .config
            .calls()
            .contains(&"transform_create_file_response")
    );
}

#[tokio::test]
async fn two_step_upload_custom_key() {
    let plan = two_step_plan(TwoStepFileUploadConfig {
        initial_request: betterlite_openai::api::file_type_records::TwoStepFileUploadRequest {
            method: String::from("POST"),
            url: String::from(API_BASE),
            headers: BTreeMap::new(),
            data: UploadData::Text(String::from("{}")),
        },
        upload_request: betterlite_openai::api::file_type_records::TwoStepFileUploadRequest {
            method: String::from("POST"),
            url: String::new(),
            headers: BTreeMap::new(),
            data: UploadData::Bytes(b"payload".to_vec()),
        },
        upload_url_location: UploadUrlLocation::Body,
        upload_url_key: Some(String::from("target")),
    });
    let mut harness = Harness::new(
        plan,
        API_BASE,
        vec![
            answer(200, &[], b"{\"upload_url\":\"http://wrong.example.test\",\"target\":\"http://right.example.test/u\"}"),
            answer(200, &[], b"{}"),
        ],
    );
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert_eq!(seen[1].url, "http://right.example.test/u");
}

#[tokio::test]
async fn upload_body_modes() {
    // presigned requests go out exactly as the transform built them
    let plan = UploadPlan::Presigned(Box::new(PresignedPlan {
        method: String::from("PUT"),
        url: String::from(UPLOAD_URL),
        headers: BTreeMap::from([(
            String::from("Content-Type"),
            String::from("application/pdf"),
        )]),
        data: UploadData::Text(String::from("pdf-bytes")),
    }));
    let mut harness = Harness::new(plan, API_BASE, vec![answer(200, &[], b"{}")]);
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert_eq!(seen[0].method, Method::PUT);
    assert_eq!(seen[0].url, UPLOAD_URL);
    assert_eq!(seen[0].body, vec![b"pdf-bytes".to_vec()]);
    assert_eq!(
        seen[0]
            .headers
            .get(CONTENT_TYPE)
            .expect("the presigned type"),
        "application/pdf"
    );
}

// a text plan uses PUT only when the provider config asks for it
#[rstest]
#[case::post(Method::POST, Method::POST)]
#[case::put(Method::PUT, Method::PUT)]
#[tokio::test]
async fn upload_text_method(#[case] method: Method, #[case] expected: Method) {
    let mut harness = Harness::build(
        Config::with_method(text_plan(), API_BASE, method),
        vec![answer(200, &[], b"{}")],
    );
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(harness.seen()[0].method, expected);
}

#[tokio::test]
async fn upload_body_modes_bytes() {
    let plan = || UploadPlan::Bytes(vec![0xff, 0x00, b'a']);
    let mut sync = Harness::new(plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    let error = sync
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    assert!(matches!(error, UploadError::Decode(_)));
    assert!(sync.seen().is_empty());

    let mut asynchronous = Harness::new(plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    asynchronous
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(asynchronous.seen()[0].body, vec![vec![0xff, 0x00, b'a']]);
}

#[tokio::test]
async fn upload_body_modes_result_context() {
    // the async handler adds no upload_url fallback, and an existing null value survives sync
    let mut asynchronous = Harness::new(text_plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    let result = asynchronous
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap();
    assert_eq!(lookup(field(&result, "params"), "upload_url"), None);

    let mut params = params();
    params.insert(String::from("upload_url"), Json::Null);
    let mut sync = Harness::new(text_plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    let result = sync.run(Mode::Sync, &create_call(), &params).await.unwrap();
    assert_eq!(
        lookup(field(&result, "params"), "upload_url"),
        Some(&Json::Null)
    );
}

#[tokio::test]
async fn upload_diagnostics() {
    let plan = UploadPlan::Presigned(Box::new(PresignedPlan {
        method: String::from("PUT"),
        url: String::from(UPLOAD_URL),
        headers: BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-secret"),
            ),
            (String::from("x-trace"), String::from("trace-1")),
        ]),
        data: UploadData::Bytes(b"pdf".to_vec()),
    }));
    let mut harness = Harness::new(plan, API_BASE, vec![answer(200, &[], b"{}")]);
    let mut call = create_call();
    call.extra_headers = IndexMap::from([(String::from("api-key"), String::from("sk-1234"))]);
    harness
        .run(Mode::Asynchronous, &call, &params())
        .await
        .unwrap();
    let details = harness.details();
    assert_eq!(field(&details, "input"), &text(""));
    assert_eq!(field(&details, "api_key"), &text(""));
    let args = field(&details, "additional_args");
    assert_eq!(field(args, "api_base"), &text(API_BASE));
    let logged_headers = field(args, "headers");
    assert_eq!(field(logged_headers, "api-key"), &text("sk****34"));
    // the presigned body's own headers are masked in the logged copy
    let body = field(args, "complete_input_dict");
    let headers = field(body, "headers");
    assert_eq!(field(headers, "Authorization"), &text("Be****et"));
    assert_eq!(field(headers, "x-trace"), &text("trace-1"));
    // the wire still carries the real credential
    let seen = harness.seen();
    assert_eq!(
        seen[0].headers.get("authorization").expect("the wire key"),
        "Bearer sk-secret"
    );
}

#[tokio::test]
async fn upload_diagnostics_media_placeholder_and_error_log() {
    let mut harness = Harness::new(
        media_plan(vec![b"body".to_vec()], None),
        API_BASE,
        vec![answer(400, &[], b"nope")],
    );
    harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap_err();
    let details = harness.details();
    assert_eq!(
        field(field(&details, "additional_args"), "complete_input_dict"),
        &text("<streaming media upload>")
    );
    let messages = harness.sink.messages(LogLevel::Error);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with("Error creating file: "));

    // a sync two-step failure logs nothing at that level
    let mut harness = Harness::new(manus_plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    assert!(harness.sink.messages(LogLevel::Error).is_empty());

    // the async two-step failure does
    let mut harness = Harness::new(manus_plan(), API_BASE, vec![answer(200, &[], b"{}")]);
    harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap_err();
    let messages = harness.sink.messages(LogLevel::Error);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Failed to get upload URL from initial request"));
}
