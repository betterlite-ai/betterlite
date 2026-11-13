#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::call::{DEFAULT_TIMEOUT, PreparedImageVariation, prepare_image_variation};
use super::client::{ClientKind, SharedSession, SuppliedClient};
use super::error::{ImageVariationSource, OpenAiImageVariationError};
use super::logging::{
    ImageVariationLogger, PostCallRecord, PreCallRecord, PrePostLogger, data_view,
};
use super::request::{
    ImageVariationOutcome, begin_image_variation, finish_image_variation, send_image_variation,
};
use super::transform::reply_identity;
use super::transport::ImageVariationTransport;
use super::wire::outbound_request;
use super::{
    ImageVariationData, ImageVariationParams, ImageVariationRequest, OpenAiImageVariationConfig,
};
use crate::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};
use crate::image_generation::decode::decode_image_reply;
use crate::image_generation::reply::{ImageGenerationReply, ImageUsage};

const NOW: i64 = 1_700_000_000;
const BOUNDARY: &str = "test-boundary";
const API_BASE: &str = "https://example.test/v1";
const OK_BODY: &str = r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#;

fn image(filename: Option<&str>, content: &str) -> ImageFile {
    ImageFile {
        filename: filename.map(str::to_owned),
        content: content.as_bytes().to_vec(),
        content_type: None,
    }
}

fn options(text: &str) -> ImageParams {
    serde_json::from_str(text).unwrap()
}

fn empty_request() -> ImageVariationRequest {
    ImageVariationRequest {
        model: None,
        image: ImageFile::default(),
        optional_params: IndexMap::new(),
        headers: None,
        api_key: None,
        api_base: None,
        organization: None,
        timeout: None,
        params: ImageVariationParams::default(),
        client: None,
        shared_session: None,
        config: None,
        boundary: String::new(),
        now_seconds: NOW,
    }
}

fn request(image: ImageFile) -> ImageVariationRequest {
    ImageVariationRequest {
        model: Some(String::from("dall-e-2")),
        image,
        api_key: Some(SecretString::from("sk-test")),
        api_base: Some(String::from(API_BASE)),
        timeout: Some(Duration::from_secs(30)),
        shared_session: Some(SharedSession),
        config: Some(OpenAiImageVariationConfig),
        boundary: String::from(BOUNDARY),
        ..empty_request()
    }
}

fn supplied(kind: ClientKind) -> SuppliedClient {
    SuppliedClient {
        kind,
        api_key: Some(SecretString::from("sk-supplied")),
        base_url: String::from("https://supplied.test/v1/"),
        timeout: Some(Duration::from_secs(45)),
        max_retries: 1,
        organization: Some(String::from("org-supplied")),
    }
}

fn object(value: &Json) -> &IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the value is an object")
        }
    }
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    name: String,
    file_name: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Recorded {
    url: String,
    parts: Vec<Part>,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    deadline: Duration,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    fn part(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.name == name)
    }

    fn names(&self) -> Vec<String> {
        self.parts.iter().map(|part| part.name.clone()).collect()
    }
}

struct FakeTransport {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    failure: bool,
    requests: Mutex<Vec<Recorded>>,
}

impl FakeTransport {
    fn answering(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| (String::from(*name), String::from(*value)))
                .collect(),
            body: body.as_bytes().to_vec(),
            failure: false,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn ok() -> Self {
        Self::answering(200, &[], OK_BODY)
    }

    fn failing() -> Self {
        Self {
            status: 0,
            headers: BTreeMap::new(),
            body: Vec::new(),
            failure: true,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ImageVariationTransport for FakeTransport {
    fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let parts = match request.body {
            RequestBody::Multipart { parts, .. } => parts
                .into_iter()
                .map(
                    |MultipartPart {
                         name,
                         file_name,
                         content_type,
                         data,
                     }| Part {
                        name,
                        file_name,
                        content_type,
                        data,
                    },
                )
                .collect(),
            RequestBody::Empty | RequestBody::Json(_) | RequestBody::Bytes { .. } => Vec::new(),
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                parts,
                headers: request.headers,
                query: request.query,
                deadline: request.deadline,
            });
        let outcome = if self.failure {
            Err(TransportError::Closed)
        } else {
            let mut headers = HeaderMap::new();
            for (name, value) in &self.headers {
                headers.insert(
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_str(value).unwrap(),
                );
            }
            Ok(UpstreamResponse {
                status: self.status,
                headers,
                body: self.body.clone(),
            })
        };
        ready(outcome)
    }
}

#[derive(Default)]
struct Recorder {
    pre: Vec<Json>,
    post: Vec<Json>,
}

fn recorded_json<T>(value: T) -> Json
where
    T: serde::Serialize,
{
    let text = serde_json::to_string(&value).unwrap();
    serde_json::from_str(&text).unwrap()
}

impl ImageVariationLogger for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(recorded_json(serde_json::json!({
            "headers": record.headers,
            "api_base": record.api_base,
            "data": data_view(&record.data),
        })));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(recorded_json(serde_json::json!({
            "headers": record.headers,
            "api_base": record.api_base,
            "original_response": record.original_response,
        })));
    }
}

async fn run(
    call: ImageVariationRequest,
    transport: &FakeTransport,
    recorder: &mut Recorder,
) -> Result<ImageVariationOutcome, OpenAiImageVariationError> {
    let prepared = begin_image_variation(call, recorder)?;
    let sent = send_image_variation(&prepared, transport).await;
    finish_image_variation(&prepared, sent, recorder)
}

fn prepared(
    call: ImageVariationRequest,
) -> Result<PreparedImageVariation, OpenAiImageVariationError> {
    prepare_image_variation(call)
}

async fn send_and_record(prepared: &PreparedImageVariation) -> Recorded {
    let transport = FakeTransport::ok();
    send_image_variation(prepared, &transport).await.unwrap();
    transport.calls().into_iter().next().unwrap()
}

#[rstest]
#[case::variation_image_part(0)]
#[case::n_size_format_user(1)]
#[case::omitted_default_fields(2)]
#[case::provider_error(3)]
#[case::binary_file_descriptor(4)]
#[case::variation_missing_config_data(5)]
#[case::variation_reply_identity(6)]
#[tokio::test]
async fn image_variations_cases(#[case] case: u8) {
    match case {
        0 => variation_image_part().await,
        1 => n_size_format_user(),
        2 => omitted_default_fields().await,
        3 => provider_error().await,
        4 => binary_file_descriptor().await,
        5 => variation_missing_config_data().await,
        6 => variation_reply_identity().await,
        _ => panic!("unknown case {case}"),
    }
}

fn transform_part() {
    let supplied_image = image(Some("cat.png"), "PNGDATA");
    let data = OpenAiImageVariationConfig::request_data(
        supplied_image.clone(),
        options(r#"{"n":2,"size":"512x512"}"#),
    )
    .unwrap();
    assert_eq!(data.image, supplied_image);
    assert_eq!(data.options.keys().collect::<Vec<_>>(), vec!["n", "size"]);
    assert!(!data.options.contains_key("model"));
    let bare =
        OpenAiImageVariationConfig::request_data(supplied_image.clone(), IndexMap::new()).unwrap();
    assert!(bare.options.is_empty());
    // an option named image cannot replace the uploaded file, so the transform yields no data
    let rejected = OpenAiImageVariationConfig::request_data(
        supplied_image,
        options(r#"{"image":"https://example.test/cat.png"}"#),
    );
    assert!(rejected.is_none());
}

async fn variation_image_part() {
    transform_part();
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let mut call = request(image(Some("cat.png"), "PNGDATA"));
    call.optional_params = options(r#"{"n":2,"size":"512x512"}"#);
    let prepared = prepared(call).unwrap();
    assert_eq!(prepared.data.options.get("n"), Some(&number(2)));
    let sent = send_image_variation(&prepared, &transport).await.unwrap();
    finish_image_variation(&prepared, Ok(sent), &mut recorder).unwrap();
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].names(), vec!["n", "size", "image"]);
    assert_eq!(calls[0].part("n").unwrap().data, b"2");
    assert_eq!(calls[0].part("size").unwrap().data, b"512x512");
    let part = calls[0].part("image").unwrap();
    assert_eq!(part.data, b"PNGDATA");
    assert_eq!(part.file_name.as_deref(), Some("cat.png"));
    assert_eq!(part.content_type.as_deref(), Some("image/png"));
    assert_eq!(calls[0].url, "https://example.test/v1/images/variations");
    assert_eq!(recorder.pre.len(), 1);
    assert_eq!(recorder.post.len(), 1);
}

fn n_size_format_user() {
    assert_eq!(
        OpenAiImageVariationConfig::SUPPORTED_PARAMS,
        ["n", "size", "response_format", "user"]
    );
    let mut existing = options(r#"{"n":1,"size":"256x256"}"#);
    let non_default = options(
        r#"{"n":2,"size":"512x512","response_format":"b64_json","user":"u1","style":"vivid"}"#,
    );
    OpenAiImageVariationConfig::map_params(&non_default, &mut existing);
    assert_eq!(existing.get("n"), Some(&number(2)));
    assert_eq!(
        existing.get("size"),
        Some(&Json::Text(String::from("512x512")))
    );
    assert_eq!(
        existing.get("response_format"),
        Some(&Json::Text(String::from("b64_json")))
    );
    assert_eq!(existing.get("user"), Some(&Json::Text(String::from("u1"))));
    // map_openai_params copies every entry, so an unsupported parameter survives the mapping
    assert_eq!(
        existing.get("style"),
        Some(&Json::Text(String::from("vivid")))
    );
    let mut untouched = options(r#"{"size":"1024x1024"}"#);
    OpenAiImageVariationConfig::map_params(&IndexMap::new(), &mut untouched);
    assert_eq!(
        untouched.get("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    let mut kept = options(r#"{"max_retries":5}"#);
    OpenAiImageVariationConfig::map_params(&options(r#"{"max_retries":7}"#), &mut kept);
    assert_eq!(kept.get("max_retries"), Some(&number(7)));
}

async fn supplied_client_fields() {
    let client = supplied(ClientKind::Sync);
    let mut call = request(image(Some("cat.png"), "PNGDATA"));
    call.client = Some(client.clone());
    call.api_key = None;
    let prepared = prepared(call).unwrap();
    assert_eq!(prepared.client.kind, ClientKind::Sync);
    assert_eq!(prepared.client.base_url, client.base_url);
    assert_eq!(prepared.client.max_retries, 1);
    assert_eq!(prepared.client.timeout, Some(Duration::from_secs(45)));
    assert!(!prepared.client.shared_session);
    assert_eq!(
        prepared.client.organization.as_deref(),
        Some("org-supplied")
    );
    let outgoing = send_and_record(&prepared).await;
    assert_eq!(
        outgoing.header("authorization").as_deref(),
        Some("Bearer sk-supplied")
    );
}

async fn built_client_fields() {
    let mut call = request(image(None, "PNGDATA"));
    call.optional_params = options(r#"{"max_retries":5}"#);
    call.organization = Some(String::from("org-test"));
    let prepared = prepared(call).unwrap();
    assert_eq!(prepared.client.kind, ClientKind::Sync);
    assert_eq!(prepared.client.base_url, "https://example.test/v1/");
    assert_eq!(prepared.client.max_retries, 5);
    assert_eq!(prepared.client.timeout, Some(Duration::from_secs(30)));
    assert!(prepared.client.shared_session);
    assert_eq!(prepared.client.organization.as_deref(), Some("org-test"));
    assert!(!prepared.data.options.contains_key("max_retries"));
    let outgoing = send_and_record(&prepared).await;
    assert_eq!(outgoing.deadline, Duration::from_secs(30));
    assert_eq!(
        outgoing.header("openai-organization").as_deref(),
        Some("org-test")
    );
    let part = outgoing.part("image").unwrap();
    assert_eq!(part.file_name.as_deref(), Some("upload"));
    assert_eq!(
        part.content_type.as_deref(),
        Some("application/octet-stream")
    );
}

async fn omitted_default_fields() {
    supplied_client_fields().await;
    built_client_fields().await;
    let mut defaulted = request(image(None, "PNGDATA"));
    defaulted.timeout = None;
    let prepared = prepared(defaulted).unwrap();
    assert_eq!(prepared.client.max_retries, 2);
    assert_eq!(prepared.client.timeout, None);
    assert_eq!(prepared.deadline, DEFAULT_TIMEOUT);
    let mut delegated = request(image(None, "PNGDATA"));
    delegated.params = ImageVariationParams { async_call: true };
    let prepared = prepared(delegated).unwrap();
    assert_eq!(prepared.client.kind, ClientKind::Async);
    assert!(prepared.async_mode);
    let sync = request(image(None, "PNGDATA"));
    assert!(!prepared(sync).unwrap().async_mode);
    let mut unkeyed = empty_request();
    unkeyed.image = image(None, "PNGDATA");
    unkeyed.config = Some(OpenAiImageVariationConfig);
    let failure = prepare_image_variation(unkeyed).unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(failure.message.contains("API key"));
}

async fn provider_error() {
    let status = FakeTransport::answering(
        429,
        &[("x-request-id", "req-429")],
        r#"{"error":{"message":"slow down"}}"#,
    );
    let mut recorder = Recorder::default();
    let failure = run(request(image(None, "PNGDATA")), &status, &mut recorder)
        .await
        .unwrap_err();
    assert_eq!(failure.status_code, 429);
    assert_eq!(
        failure.headers,
        Some(ImageHeaders::from([(
            String::from("x-request-id"),
            String::from("req-429"),
        )]))
    );
    assert!(failure.message.contains("slow down"));
    // the failed call still ran pre_call, and never ran post_call
    assert_eq!(recorder.pre.len(), 1);
    assert!(recorder.post.is_empty());
    let transport = FakeTransport::failing();
    let failure = run(request(image(None, "PNGDATA")), &transport, &mut recorder)
        .await
        .unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert_eq!(failure.headers, None);
    let mut delegated = request(image(None, "PNGDATA"));
    delegated.params = ImageVariationParams { async_call: true };
    let failure = run(delegated, &FakeTransport::failing(), &mut recorder)
        .await
        .unwrap_err();
    assert_eq!(failure.status_code, 500);
    // the SDK parses the reply as ImagesResponse, so an error object is a 500, not a 422
    let error_object = FakeTransport::answering(200, &[], r#"{"error":{"message":"boom"}}"#);
    let failure = run(
        request(image(None, "PNGDATA")),
        &error_object,
        &mut recorder,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(matches!(*failure.source, ImageVariationSource::Reply(_)));
}

async fn binary_file_descriptor() {
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let outcome = run(
        request(image(Some("cat.png"), "PNGDATA")),
        &transport,
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(outcome.reply.created, 7);
    let calls = transport.calls();
    let part = calls[0].part("image").unwrap();
    assert_eq!(part.name, "image");
    assert_eq!(part.data, b"PNGDATA");
    assert_eq!(part.file_name.as_deref(), Some("cat.png"));
    assert_eq!(part.content_type.as_deref(), Some("image/png"));
    assert!(recorder.post.is_empty());
    let typed = ImageFile {
        filename: Some(String::from("pic.webp")),
        content: b"WEBP".to_vec(),
        content_type: Some(String::from("image/webp")),
    };
    let prepared = prepared(request(typed)).unwrap();
    let outgoing = send_and_record(&prepared).await;
    let part = outgoing.part("image").unwrap();
    assert_eq!(part.file_name.as_deref(), Some("pic.webp"));
    assert_eq!(part.content_type.as_deref(), Some("image/webp"));
    assert_eq!(part.data, b"WEBP");
}

async fn variation_missing_config_data() {
    let mut missing = request(image(None, "PNGDATA"));
    missing.config = None;
    let failure = prepare_image_variation(missing).unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(failure.message.contains("provider not found: openai"));
    assert!(matches!(
        *failure.source,
        ImageVariationSource::MissingConfig { provider: "openai" }
    ));
    let mut false_data = request(image(None, "PNGDATA"));
    false_data.optional_params = options(r#"{"image":"not-a-file"}"#);
    let failure = prepare_image_variation(false_data).unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(failure.message.contains("data field is required"));
    assert!(matches!(*failure.source, ImageVariationSource::MissingData));
    // neither seam reaches the network or the logging hooks
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let mut missing = request(image(None, "PNGDATA"));
    missing.config = None;
    let failure = run(missing, &transport, &mut recorder).await.unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(transport.calls().is_empty());
    assert!(recorder.pre.is_empty());
}

async fn variation_reply_identity() {
    let body = r#"{"created":7,"data":[{"b64_json":"YQ=="}],"quality":"medium","size":"1024x1024","usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":1,"text_tokens":3},"output_tokens":8,"total_tokens":12}}"#;
    let transport = FakeTransport::answering(200, &[("x-request-id", "req-1")], body);
    let mut recorder = Recorder::default();
    let outcome = run(request(image(None, "PNGDATA")), &transport, &mut recorder)
        .await
        .unwrap();
    reply_fields(&outcome.reply);
    let dumped = serde_json::to_string(&outcome.reply).unwrap();
    assert!(!dumped.contains("\"prompt_tokens\""));
    assert!(!dumped.contains("\"prompt_tokens_details\""));
    assert!(dumped.contains(r#""output_tokens_details":null"#));
    let unchanged = decode_image_reply(body.as_bytes(), NOW).unwrap();
    assert_eq!(outcome.reply.data, unchanged.data);
    let logged = object(&recorder.post[0]);
    let original = object(logged.get("original_response").unwrap());
    assert_eq!(original.get("created"), Some(&number(7)));
    let data = match original.get("data") {
        Some(Json::List(items)) => items,
        _ => panic!("the logged reply carries data"),
    };
    assert!(object(&data[0]).get("provider_specific_fields").is_none());
    assert_eq!(recorder.pre.len(), 1);
    assert_eq!(recorder.post.len(), 1);
    let bodyless = FakeTransport::answering(200, &[], r#"{"data":[]}"#);
    let failure = run(request(image(None, "PNGDATA")), &bodyless, &mut recorder)
        .await
        .unwrap_err();
    assert_eq!(failure.status_code, 500);
}

fn reply_fields(reply: &ImageGenerationReply) {
    assert_eq!(reply.created, 7);
    assert_eq!(reply.data[0].b64_json.as_deref(), Some("YQ=="));
    assert_eq!(reply.quality.as_deref(), Some("medium"));
    assert_eq!(reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(reply.usage.input_tokens, 4);
    assert_eq!(reply.usage.total_tokens, 12);
    assert_eq!(reply.usage.prompt_tokens, None);
    assert_eq!(reply.usage.completion_tokens, None);
    assert_eq!(
        reply.usage.extra.get("output_tokens_details"),
        Some(&Json::Null)
    );
    assert!(reply.usage.extra.get("prompt_tokens_details").is_none());
}

#[derive(Default)]
struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

fn logging() -> (Redactor, LoggingState, Sink) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(
            r#"{"model":"dall-e-2","call_type":"image_variation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default(),
    )
    .unwrap();
    (redactor, state, Sink::default())
}

#[tokio::test]
async fn image_variations_logs_the_pre_call_then_the_post_call() {
    let (redactor, mut state, sink) = logging();
    let snapshots = Mutex::new(Vec::new());
    let logger = |details: &Json| {
        snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    let transport = FakeTransport::ok();
    let prepared = {
        let mut hooks = PrePostLogger {
            state: &mut state,
            env: &env,
        };
        let mut call = request(image(Some("cat.png"), "PNGDATA"));
        call.headers = Some(ImageHeaders::from([(
            String::from("x-caller"),
            String::from("yes"),
        )]));
        begin_image_variation(call, &mut hooks).unwrap()
    };
    let sent = send_image_variation(&prepared, &transport).await;
    {
        let mut hooks = PrePostLogger {
            state: &mut state,
            env: &env,
        };
        finish_image_variation(&prepared, sent, &mut hooks).unwrap();
    }
    let recorded = snapshots
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        object(&recorded[0]).get("log_event_type"),
        Some(&Json::Text(String::from("pre_api_call")))
    );
    assert_eq!(
        object(&recorded[1]).get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let pre = object(&recorded[0]);
    assert_eq!(pre.get("input"), Some(&Json::Text(String::new())));
    let arguments = object(pre.get("additional_args").unwrap());
    assert_eq!(
        arguments.get("api_base"),
        Some(&Json::Text(String::from("https://example.test/v1/")))
    );
    let headers = object(arguments.get("headers").unwrap());
    assert!(headers.contains_key("x-caller"));
    let body = object(arguments.get("complete_input_dict").unwrap());
    let data = object(body.get("data").unwrap());
    assert!(data.get("size").is_none());
    let image_view = object(data.get("image").unwrap());
    assert_eq!(
        image_view.get("filename"),
        Some(&Json::Text(String::from("cat.png")))
    );
    assert_eq!(image_view.get("size"), Some(&number(7)));
    let post = object(&recorded[1]);
    assert_eq!(post.get("input"), Some(&Json::Null));
    let original = object(post.get("original_response").unwrap());
    assert_eq!(original.get("created"), Some(&number(7)));
}

#[test]
fn outbound_request_carries_the_sdk_extras() {
    let mut call = request(image(None, "PNGDATA"));
    call.optional_params = options(
        r#"{"n":2,"size":"","extra_headers":{"x-old":"no","x-test":"yes"},"extra_query":{"api-version":"1"},"extra_body":{"seed":7}}"#,
    );
    let prepared = prepare_image_variation(call).unwrap();
    let outbound = outbound_request(&prepared).unwrap();
    assert_eq!(
        outbound.url.as_str(),
        "https://example.test/v1/images/variations"
    );
    assert_eq!(
        outbound.query,
        vec![(String::from("api-version"), String::from("1"))]
    );
    assert_eq!(
        outbound
            .headers
            .get("x-test")
            .and_then(|value| value.to_str().ok()),
        Some("yes")
    );
    let RequestBody::Multipart { parts, boundary } = outbound.body else {
        panic!("the variation request is multipart");
    };
    assert_eq!(boundary, BOUNDARY);
    let names: Vec<_> = parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(names, vec!["n", "seed", "image"]);
    assert_eq!(parts[0].data, b"2");
    assert_eq!(parts[1].data, b"7");
    assert_eq!(parts[2].data, b"PNGDATA");
}

#[tokio::test]
async fn a_supplied_client_of_the_other_kind_is_refused() {
    let mut call = request(image(None, "PNGDATA"));
    call.client = Some(supplied(ClientKind::Async));
    let failure = prepare_image_variation(call).unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(matches!(
        *failure.source,
        ImageVariationSource::ClientKind {
            supplied: "async",
            needed: "sync"
        }
    ));
}

#[test]
fn the_error_class_keeps_the_message_and_headers() {
    let error = OpenAiImageVariationError::wrapped(ImageVariationSource::Status {
        status: 400,
        message: String::from("bad request"),
        headers: Some(ImageHeaders::from([(String::from("x"), String::from("y"))])),
    });
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "bad request");
    assert_eq!(
        error.headers.unwrap().get("x").map(String::as_str),
        Some("y")
    );
    let wrapped = OpenAiImageVariationError::wrapped(ImageVariationSource::MaxRetries);
    assert_eq!(wrapped.status_code, 500);
    assert_eq!(wrapped.headers, None);
}

#[test]
fn the_identity_reply_keeps_the_sdk_shape() {
    let reply = decode_image_reply(br#"{"created":7,"data":[]}"#, NOW).unwrap();
    let identity = reply_identity(reply);
    assert_eq!(identity.created, 7);
    let dumped = serde_json::to_string(&identity).unwrap();
    assert!(dumped.contains(r#""background":null"#));
    assert!(dumped.contains(r#""input_tokens":0"#));
    assert!(!dumped.contains("output_tokens_details"));
    let with_vendor = reply_identity(ImageGenerationReply {
        created: 7,
        data: Vec::new(),
        usage: ImageUsage {
            input_tokens: 1,
            input_tokens_details: Default::default(),
            output_tokens: 2,
            total_tokens: 3,
            prompt_tokens: Some(1),
            completion_tokens: Some(2),
            extra: IndexMap::from([(String::from("vendor"), Json::Bool(true))]),
        },
        background: None,
        quality: None,
        output_format: None,
        size: None,
    });
    let dumped = serde_json::to_string(&with_vendor).unwrap();
    assert!(!dumped.contains("prompt_tokens"));
    assert!(!dumped.contains("vendor"));
    assert!(dumped.contains(r#""output_tokens_details":null"#));
}

#[test]
fn image_variation_params_default_to_the_sync_call() {
    assert!(!ImageVariationParams::default().async_call);
    assert_eq!(ClientKind::Sync.label(), "sync");
    assert_eq!(ClientKind::Async.label(), "async");
    let data = ImageVariationData {
        image: image(Some("cat.png"), "PNGDATA"),
        options: options(r#"{"n":2}"#),
    };
    let view = data_view(&data);
    let inner = object(object(&view).get("data").unwrap());
    assert_eq!(inner.get("n"), Some(&number(2)));
    assert_eq!(
        object(inner.get("image").unwrap()).get("size"),
        Some(&number(7))
    );
}

async fn serve_variation_reply(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !received.windows(8).any(|window| window == b"PNGDATA\r") {
        let read = stream.read(&mut chunk).await.unwrap();
        received.extend_from_slice(&chunk[..read]);
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-request-id: req-9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{OK_BODY}",
        OK_BODY.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(received).unwrap()
}

#[tokio::test]
async fn proxied_client_posts_multipart_to_the_variations_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut local = request(image(Some("cat.png"), "PNGDATA"));
    local.api_base = Some(format!("http://{address}/v1"));
    local.boundary = String::from("wire-boundary");
    let prepared = prepare_image_variation(local).unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let (head, sent) = tokio::join!(
        serve_variation_reply(listener),
        send_image_variation(&prepared, &client)
    );
    let outcome = sent.unwrap();
    let head = head.to_lowercase();
    assert!(
        head.starts_with("post /v1/images/variations http/1.1"),
        "{head}"
    );
    assert!(head.contains("authorization: bearer sk-test"), "{head}");
    assert!(
        head.contains("content-type: multipart/form-data; boundary=wire-boundary"),
        "{head}"
    );
    assert!(
        head.contains("name=\"image\"; filename=\"cat.png\""),
        "{head}"
    );
    assert!(head.contains("pngdata"), "{head}");
    assert_eq!(outcome.reply.created, 7);
}
