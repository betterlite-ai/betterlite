use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
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
use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use crate::api::file_http_metadata::config::{
    EnvironmentRequest, FileParams, FileRequest, FileResponse, Headers, ListRequest,
    MetadataConfig, NextPageRequest, PreparedRequest, ReplyContext,
};
use crate::api::file_http_metadata::error::MetadataError;
use crate::api::file_http_metadata::service::{
    MetadataCall, MetadataDeps, MetadataTarget, MetadataValue, prepare, send,
};
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest, WriteTransport,
};

pub(crate) const BASE: &str = "http://provider.test";
pub(crate) const FILE_ID: &str = "file_f";
pub(crate) const SECRET: &str = "sk-provider";
pub(crate) const TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The logging state a production request runs pre_call against, with the logger callback recording its turn
pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    pub(crate) state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"files","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    pub(crate) async fn execute<C, S>(
        &mut self,
        deps: &MetadataDeps<'_, C, S>,
        mode: ClientKind,
        call: &MetadataCall<'_>,
        events: &Events,
    ) -> Result<MetadataValue, MetadataError>
    where
        C: MetadataConfig,
        S: ClientSource<Client: WriteTransport>,
    {
        let logger = |_details: &Json| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: Some(&logger),
            now: self.now,
        };
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        let prepared = prepare(deps, mode, call, &mut log)?;
        send(prepared, &mut log).await
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }

    pub(crate) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }
}

pub(crate) async fn run<C, S>(
    fixture: &mut Fixture,
    deps: &MetadataDeps<'_, C, S>,
    mode: ClientKind,
    call: &MetadataCall<'_>,
    events: &Events,
) -> Result<MetadataValue, MetadataError>
where
    C: MetadataConfig,
    S: ClientSource<Client: WriteTransport>,
{
    fixture.execute(deps, mode, call, events).await
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            http::HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

pub(crate) enum Answer {
    Reply(Reply),
    Fail(TransportError),
}

#[derive(Clone)]
pub(crate) struct Wire {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<Arc<WriteRequest>>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn new(answers: Vec<Answer>, events: &Events) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Arc<WriteRequest>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl WriteTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(request));
        let next = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let answer = match next {
            Some(Answer::Reply(reply)) => Ok(UpstreamResponse {
                status: reply.status,
                headers: reply.headers,
                body: reply.body,
            }),
            Some(Answer::Fail(error)) => Err(error),
            None => Err(TransportError::Closed),
        };
        ready(answer)
    }
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<Option<bool>>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Option<bool>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.wire.clone())
    }
}

pub(crate) const fn metadata_deps<'deps>(
    config: &'deps RecordingConfig,
    source: &'deps Source,
    supplied: Option<SuppliedClient<'deps, Wire>>,
) -> MetadataDeps<'deps, RecordingConfig, Source> {
    MetadataDeps {
        config,
        source,
        supplied,
    }
}

pub(crate) const fn target<'call>(
    litellm_params: &'call FileParams,
    extra_headers: Option<&'call BTreeMap<String, String>>,
    timeout: Option<Duration>,
) -> MetadataTarget<'call> {
    MetadataTarget {
        litellm_params,
        extra_headers,
        ssl_verify: None,
        timeout,
    }
}

pub(crate) fn litellm_params(base: &str, api_key: &str) -> FileParams {
    let mut params = FileParams::new();
    params.insert(
        "api_base".to_owned(),
        serde_json::to_value(base).expect("the base is text"),
    );
    params.insert(
        "api_key".to_owned(),
        serde_json::to_value(api_key).expect("the key is text"),
    );
    params
}

pub(crate) fn litellm_params_with(base: &str, api_key: &str, extra: &[(&str, &str)]) -> FileParams {
    let mut params = litellm_params(base, api_key);
    for (name, value) in extra {
        params.insert(
            (*name).to_owned(),
            serde_json::to_value(*value).expect("the extra value is text"),
        );
    }
    params
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn file_object(id: &str) -> FileObject {
    FileObject {
        id: id.to_owned(),
        bytes: 12,
        created_at: 100,
        filename: format!("{id}.jsonl"),
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: Some(FileStatus::Uploaded),
        expires_at: Some(200),
        status_details: None,
    }
}

pub(crate) fn files(prefix: &str, count: usize) -> Vec<FileObject> {
    (0..count)
        .map(|index| file_object(&format!("{prefix}-{index}")))
        .collect()
}

pub(crate) fn deleted() -> FileDeleted {
    FileDeleted {
        id: FILE_ID.to_owned(),
        object: FileKind::File,
        deleted: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Validate,
    Request,
    Response,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Failure {
    stage: Stage,
    status: u16,
    message: &'static str,
}

impl Failure {
    pub(crate) const fn at(stage: Stage, status: u16, message: &'static str) -> Self {
        Self {
            stage,
            status,
            message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Validated {
    pub(crate) api_key: Option<String>,
    pub(crate) model: String,
    pub(crate) messages: usize,
    pub(crate) optional_empty: bool,
    pub(crate) headers: Vec<String>,
    pub(crate) incoming_validation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requested {
    pub(crate) file_id: Option<String>,
    pub(crate) purpose: Option<String>,
    pub(crate) optional_empty: bool,
    pub(crate) litellm_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) additional_args: bool,
    pub(crate) previous_page: Option<Json>,
}

// The provider config these tests hand the layer: it records every call and answers with the pages and next requests set on it
pub(crate) struct RecordingConfig {
    base: String,
    failure: Option<Failure>,
    pages: Mutex<VecDeque<Vec<FileObject>>>,
    nexts: Mutex<VecDeque<Option<BTreeMap<String, String>>>>,
    validates: Mutex<Vec<Validated>>,
    requested: Mutex<Vec<Requested>>,
    observed: Mutex<Vec<Observed>>,
    errors: Mutex<Vec<(String, u16, HeaderMap)>>,
    urls: Mutex<Vec<Option<String>>>,
    next_calls: AtomicUsize,
    events: Events,
    number_query: AtomicBool,
}

impl RecordingConfig {
    pub(crate) fn new(base: &str, events: &Events) -> Self {
        Self {
            base: base.to_owned(),
            failure: None,
            pages: Mutex::default(),
            nexts: Mutex::default(),
            validates: Mutex::default(),
            requested: Mutex::default(),
            observed: Mutex::default(),
            errors: Mutex::default(),
            urls: Mutex::default(),
            next_calls: AtomicUsize::new(0),
            events: events.clone(),
            number_query: AtomicBool::new(false),
        }
    }

    pub(crate) fn failing(base: &str, failure: Failure, events: &Events) -> Self {
        let mut config = Self::new(base, events);
        config.failure = Some(failure);
        config
    }

    pub(crate) fn set_pages(&self, pages: Vec<Vec<FileObject>>) {
        *self.pages.lock().unwrap_or_else(PoisonError::into_inner) = VecDeque::from(pages);
    }

    pub(crate) fn set_nexts(&self, nexts: Vec<Option<BTreeMap<String, String>>>) {
        *self.nexts.lock().unwrap_or_else(PoisonError::into_inner) = VecDeque::from(nexts);
    }

    pub(crate) fn set_number_query(&self) {
        self.number_query.store(true, Ordering::Relaxed);
    }

    pub(crate) fn validates(&self) -> Vec<Validated> {
        self.validates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn requested(&self) -> Vec<Requested> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn observed(&self) -> Vec<Observed> {
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, HeaderMap)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn urls(&self) -> Vec<Option<String>> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn next_calls(&self) -> usize {
        self.next_calls.load(Ordering::Relaxed)
    }

    pub(crate) fn nexts_remaining(&self) -> usize {
        self.nexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn failure_at(&self, stage: Stage) -> Option<BaseLlmException> {
        let failure = self.failure?;
        if failure.stage != stage {
            return None;
        }
        Some(BaseLlmException {
            status_code: failure.status,
            message: failure.message.to_owned(),
            headers: HeaderMap::new(),
            status_code_is_synthesized: false,
        })
    }

    fn record_request(
        &self,
        file_id: Option<String>,
        purpose: Option<String>,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<(), BaseLlmException> {
        self.events.push("request");
        if let Some(error) = self.failure_at(Stage::Request) {
            return Err(error);
        }
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Requested {
                file_id,
                purpose,
                optional_empty: optional_params.is_empty(),
                litellm_keys: litellm_params.keys().cloned().collect(),
            });
        Ok(())
    }

    fn observe(&self, model_call_details: &mut IndexMap<String, Json>) {
        let mut observed = self.observed.lock().unwrap_or_else(PoisonError::into_inner);
        let index = observed.len();
        observed.push(Observed {
            additional_args: model_call_details.contains_key("additional_args"),
            previous_page: model_call_details.get("page").cloned(),
        });
        drop(observed);
        model_call_details.insert("page".to_owned(), Json::Number(Number::from(index)));
    }

    fn check_response(&self, response: &FileResponse) -> Result<(), BaseLlmException> {
        if let Some(error) = self.failure_at(Stage::Response) {
            return Err(error);
        }
        if response.status_code < 400 {
            return Ok(());
        }
        let message = from_utf8(&response.body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", response.body.len()),
            ToOwned::to_owned,
        );
        Err(BaseLlmException {
            status_code: response.status_code,
            message,
            headers: response
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                .collect(),
            status_code_is_synthesized: false,
        })
    }

    fn query(&self, name: &str, value: &str) -> FileParams {
        let mut query = FileParams::new();
        let chosen = if self.number_query.load(Ordering::Relaxed) {
            serde_json::to_value(1_u32).expect("the number query value")
        } else {
            serde_json::to_value(value).expect("the text query value")
        };
        query.insert(name.to_owned(), chosen);
        query
    }

    fn recorded(&self, item: RequestItem<'_>) -> Result<PreparedRequest, BaseLlmException> {
        match item {
            RequestItem::File(request) => {
                let file_id = request.file_id.to_owned();
                let prepared = (
                    format!("{}/files/{file_id}", self.base),
                    self.query("view", "full"),
                );
                self.record_request(
                    Some(file_id),
                    None,
                    request.optional_params,
                    request.litellm_params,
                )?;
                Ok(prepared)
            }
            RequestItem::List(request) => {
                let purpose = request.purpose.map(str::to_owned);
                let query = request
                    .purpose
                    .map_or_else(FileParams::new, |name| self.query("purpose", name));
                self.record_request(
                    None,
                    purpose,
                    request.optional_params,
                    request.litellm_params,
                )?;
                Ok((format!("{}/files", self.base), query))
            }
        }
    }

    fn respond<T, F>(
        &self,
        raw_response: &FileResponse,
        model_call_details: &mut IndexMap<String, Json>,
        answer: F,
    ) -> Result<T, BaseLlmException>
    where
        F: FnOnce() -> T,
    {
        self.events.push("response");
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(raw_response.url.clone());
        self.observe(model_call_details);
        self.check_response(raw_response)?;
        Ok(answer())
    }

    fn pop_page(&self) -> Vec<FileObject> {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }
}

enum RequestItem<'call> {
    File(FileRequest<'call>),
    List(ListRequest<'call>),
}

impl MetadataConfig for RecordingConfig {
    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, BaseLlmException> {
        self.events.push("validate");
        let index = self
            .validates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        self.validates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Validated {
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                model: request.model.to_owned(),
                messages: request.messages.len(),
                optional_empty: request.optional_params.is_empty(),
                headers: request.headers.keys().cloned().collect(),
                incoming_validation: request.headers.get("x-validation").cloned(),
            });
        if let Some(error) = self.failure_at(Stage::Validate) {
            return Err(error);
        }
        let mut headers = request.headers;
        headers.insert(
            "authorization".to_owned(),
            format!(
                "Bearer {}",
                request.api_key.map_or("", |key| key.expose_secret())
            ),
        );
        let number = index.checked_add(1).expect("the validation number");
        headers.insert("x-validation".to_owned(), format!("{number}"));
        Ok(headers)
    }

    fn transform_retrieve_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.recorded(RequestItem::File(request))
    }

    fn transform_retrieve_file_response(
        &self,
        raw_response: &FileResponse,
        context: ReplyContext<'_>,
    ) -> Result<FileObject, BaseLlmException> {
        self.respond(raw_response, context.model_call_details, || {
            file_object(FILE_ID)
        })
    }

    fn transform_delete_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.recorded(RequestItem::File(request))
    }

    fn transform_delete_file_response(
        &self,
        raw_response: &FileResponse,
        context: ReplyContext<'_>,
    ) -> Result<FileDeleted, BaseLlmException> {
        self.respond(raw_response, context.model_call_details, deleted)
    }

    fn transform_list_files_request(
        &self,
        request: ListRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.recorded(RequestItem::List(request))
    }

    fn transform_list_files_next_request(
        &self,
        _raw_response: &FileResponse,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Option<NextPageRequest> {
        self.next_calls.fetch_add(1, Ordering::Relaxed);
        self.nexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(None)
            .map(|query| (self.base.clone(), query))
    }

    fn transform_list_files_response(
        &self,
        raw_response: &FileResponse,
        context: ReplyContext<'_>,
    ) -> Result<Vec<FileObject>, BaseLlmException> {
        self.respond(raw_response, context.model_call_details, || self.pop_page())
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.events.push("error");
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

pub(crate) const PATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_secs(5),
    ..TransportLimits::DEFAULT
};

// The per-call timeout the delayed fixture outlasts, so only a send that carries it times out
pub(crate) const SHORT: Duration = Duration::from_millis(10);

pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        let policy = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
                .expect("the proxy policy resolves"),
        );
        Self {
            client: ProxiedClient::build(base_builder().http1_only(), &policy, limits)
                .expect("the proxied client builds"),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}
