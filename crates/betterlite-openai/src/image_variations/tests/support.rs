use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::context::LogContext;
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
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};
use crate::image_variations::call::{PreparedImageVariation, prepare_image_variation};
use crate::image_variations::client::{ImageVariationClient, SharedSession};
use crate::image_variations::error::OpenAiImageVariationError;
use crate::image_variations::logging::{
    ImageVariationLogger, PostCallRecord, PreCallRecord, data_view,
};
use crate::image_variations::request::{
    ImageVariationOutcome, begin_image_variation, finish_image_variation, send_image_variation,
};
use crate::image_variations::transport::ImageVariationTransport;
use crate::image_variations::{
    ImageVariationParams, ImageVariationRequest, OpenAiImageVariationConfig,
};
use betterlite_http::client_kind::ClientKind;

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const BOUNDARY: &str = "test-boundary";
pub(super) const OK_BODY: &str = r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#;
const API_BASE: &str = "https://example.test/v1";

pub(super) fn image(filename: Option<&str>, content: &str) -> ImageFile {
    ImageFile {
        filename: filename.map(str::to_owned),
        content: content.as_bytes().to_vec(),
        content_type: None,
    }
}

pub(super) fn options(text: &str) -> ImageParams {
    serde_json::from_str(text).unwrap()
}

pub(super) fn empty_request() -> ImageVariationRequest {
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

pub(super) fn request(image: ImageFile) -> ImageVariationRequest {
    let mut call = empty_request();
    call.model = Some(String::from("dall-e-2"));
    call.image = image;
    call.api_key = Some(SecretString::from("sk-test"));
    call.api_base = Some(String::from(API_BASE));
    call.timeout = Some(Duration::from_secs(30));
    call.shared_session = Some(SharedSession);
    call.config = Some(OpenAiImageVariationConfig);
    call.boundary = String::from(BOUNDARY);
    call
}

pub(super) fn supplied(kind: ClientKind) -> ImageVariationClient {
    ImageVariationClient {
        kind,
        api_key: Some(SecretString::from("sk-supplied")),
        base_url: String::from("https://supplied.test/v1/"),
        timeout: Some(Duration::from_secs(45)),
        max_retries: 1,
        organization: Some(String::from("org-supplied")),
        shared_session: true,
    }
}

pub(super) fn object(value: &Json) -> &IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the value is an object")
        }
    }
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Part {
    pub(super) name: String,
    pub(super) file_name: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub(super) url: String,
    parts: Vec<Part>,
    headers: HeaderMap,
    pub(super) query: Vec<(String, String)>,
    pub(super) deadline: Duration,
}

impl Recorded {
    pub(super) fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    pub(super) fn part(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.name == name)
    }

    pub(super) fn names(&self) -> Vec<String> {
        self.parts.iter().map(|part| part.name.clone()).collect()
    }

    pub(super) fn values(&self) -> Vec<&[u8]> {
        self.parts.iter().map(|part| part.data.as_slice()).collect()
    }
}

pub(super) struct FakeTransport {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    failure: bool,
    requests: Mutex<Vec<Recorded>>,
}

impl FakeTransport {
    pub(super) fn answering(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::answering_bytes(status, headers, body.as_bytes().to_vec())
    }

    pub(super) fn answering_bytes(status: u16, headers: &[(&str, &str)], body: Vec<u8>) -> Self {
        Self {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| (String::from(*name), String::from(*value)))
                .collect(),
            body,
            failure: false,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn ok() -> Self {
        Self::answering(200, &[], OK_BODY)
    }

    pub(super) fn failing() -> Self {
        Self {
            status: 0,
            headers: BTreeMap::new(),
            body: Vec::new(),
            failure: true,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn recorded_parts(body: RequestBody) -> Vec<Part> {
    match body {
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
    }
}

fn recorded_headers(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut recorded = HeaderMap::new();
    for (name, value) in headers {
        recorded.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    recorded
}

impl ImageVariationTransport for FakeTransport {
    fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let parts = recorded_parts(request.body);
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
            Ok(UpstreamResponse {
                status: self.status,
                headers: recorded_headers(&self.headers),
                body: self.body.clone(),
            })
        };
        ready(outcome)
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<Json>,
    pub(super) post: Vec<Json>,
}

#[derive(serde::Serialize)]
struct PreRecord<'record> {
    headers: &'record Option<ImageHeaders>,
    api_base: &'record Option<String>,
    data: IndexMap<String, Json>,
}

#[derive(serde::Serialize)]
struct PostRecord<'record> {
    headers: &'record Option<ImageHeaders>,
    api_base: &'record Option<String>,
    original_response: &'record Json,
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
        self.pre.push(recorded_json(PreRecord {
            headers: &record.headers,
            api_base: &record.api_base,
            data: data_view(&record.data),
        }));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(recorded_json(PostRecord {
            headers: &record.headers,
            api_base: &record.api_base,
            original_response: &record.original_response,
        }));
    }
}

pub(super) async fn run(
    call: ImageVariationRequest,
    transport: &FakeTransport,
    recorder: &mut Recorder,
) -> Result<ImageVariationOutcome, OpenAiImageVariationError> {
    let prepared = begin_image_variation(call, recorder)?;
    let sent = send_image_variation(&prepared, transport).await;
    finish_image_variation(&prepared, sent, recorder)
}

pub(super) async fn failure_of(
    call: ImageVariationRequest,
    transport: &FakeTransport,
    recorder: &mut Recorder,
) -> OpenAiImageVariationError {
    run(call, transport, recorder).await.unwrap_err()
}

pub(super) fn prepared(
    call: ImageVariationRequest,
) -> Result<PreparedImageVariation, OpenAiImageVariationError> {
    prepare_image_variation(call)
}

pub(super) async fn send_and_record(prepared: &PreparedImageVariation) -> Recorded {
    let transport = FakeTransport::ok();
    send_image_variation(prepared, &transport).await.unwrap();
    transport.calls().into_iter().next().unwrap()
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

type Snapshots = Arc<Mutex<Vec<Json>>>;
type SnapshotLogger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Sync>;

fn logging() -> (Redactor, LoggingState, Sink, Snapshots, SnapshotLogger) {
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
        Duration::default,
    )
    .unwrap();
    let snapshots: Snapshots = Arc::default();
    let recorded = Arc::clone(&snapshots);
    let logger: SnapshotLogger = Box::new(move |details: &Json| {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    });
    (redactor, state, Sink::default(), snapshots, logger)
}

pub(super) fn logged_records(call: ImageVariationRequest) -> Vec<Json> {
    let (redactor, mut state, sink, snapshots, logger) = logging();
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
        logger_fn: Some(logger.as_ref()),
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    let prepared = {
        let mut hooks = LogContext {
            state: &mut state,
            env: &env,
        };
        begin_image_variation(call, &mut hooks).unwrap()
    };
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let sent = runtime.block_on(send_image_variation(&prepared, &FakeTransport::ok()));
    let mut hooks = LogContext {
        state: &mut state,
        env: &env,
    };
    finish_image_variation(&prepared, sent, &mut hooks).unwrap();
    snapshots
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
