use std::error::Error;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::vector_store_http_write::error::{
    ClientError, HeaderMap as FactsHeaders,
};
use betterlite_core::api::vector_store_http_write::service::{WriteRequest, WriteTransport};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
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
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageParams, ImageResponse,
};
use betterlite_openai::error::ProviderError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::super::error::{ImageEditHttpError, RequestError};
use super::super::service::{
    ClientKind, ClientSource, ImageEditCall, ImageEditDeps, ImageEditOutcome, LogContext,
    SuppliedClient,
};
use super::super::{async_image_edit_handler, image_edit_handler};
use crate::api::image_contracts::edit::{
    BaseImageEditConfig, ImageEditEnvironment, ImageEditReply, ImageEditRequest,
};
use crate::api::image_contracts::error::ImageContractError;
use crate::api::image_contracts::shared::base_exception;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(super) fn params(text: &str) -> ImageParams {
    serde_json::from_str(text).expect("the params fixture parses")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> ImageHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn fact_headers(pairs: &[(&str, &str)]) -> FactsHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

// Everything the handler asks the provider config, in the order it asks
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Event {
    Environment {
        api_key: Option<String>,
        api_base: Option<String>,
        model: String,
        headers: ImageHeaders,
    },
    Url {
        api_base: Option<String>,
        api_key: Option<String>,
    },
    Transform {
        mode: ClientKind,
        model: String,
        prompt: Option<String>,
        image: Option<ImageFile>,
        options: ImageParams,
        headers: ImageHeaders,
    },
    Finalize {
        url: String,
        data: ImageParams,
    },
    Respond {
        status: u16,
    },
    Classify {
        message: String,
        status_code: u16,
        headers: ImageHeaders,
    },
}

// How the fixture answers one step of the call: every step but the refusals behaves normally
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behavior {
    Answer,
    RefuseUrl,
    RefuseTransform,
    RefuseResponse,
    NestFinalized,
}

pub(super) struct RecordingConfig {
    events: Mutex<Vec<Event>>,
    multipart: bool,
    base: String,
    behavior: Behavior,
}

impl RecordingConfig {
    pub(super) fn new(base: &str) -> Self {
        Self {
            events: Mutex::default(),
            multipart: true,
            base: base.to_owned(),
            behavior: Behavior::Answer,
        }
    }

    pub(super) const fn without_multipart(mut self) -> Self {
        self.multipart = false;
        self
    }

    pub(super) const fn failing_url(mut self) -> Self {
        self.behavior = Behavior::RefuseUrl;
        self
    }

    pub(super) const fn failing_transform(mut self) -> Self {
        self.behavior = Behavior::RefuseTransform;
        self
    }

    pub(super) const fn failing_response(mut self) -> Self {
        self.behavior = Behavior::RefuseResponse;
        self
    }

    pub(super) const fn finalizing_nested(mut self) -> Self {
        self.behavior = Behavior::NestFinalized;
        self
    }

    pub(super) fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn transformed(
        &self,
        mode: ClientKind,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), FixtureError> {
        self.record(Event::Transform {
            mode,
            model: request.model.to_owned(),
            prompt: request.prompt.map(str::to_owned),
            image: request.image.cloned(),
            options: request.optional_params.clone(),
            headers: request.headers.clone(),
        });
        if self.behavior == Behavior::RefuseTransform {
            return Err(FixtureError::Contract(ImageContractError::NotImplemented {
                message: "the fixture transform failed",
            }));
        }
        let mut data = ImageParams::new();
        if let Some(prompt) = request.prompt {
            data.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
        }
        let files: ImageFiles = request
            .image
            .map(|image| vec![(String::from("image"), image.clone())])
            .unwrap_or_default();
        Ok((data, files))
    }
}

#[async_trait]
impl BaseImageEditConfig for RecordingConfig {
    type Error = FixtureError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _image_edit_optional_params: &ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        ImageParams::new()
    }

    fn transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error> {
        self.transformed(ClientKind::Sync, request)
    }

    async fn async_transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error> {
        self.transformed(ClientKind::Asynchronous, request)
    }

    fn transform_image_edit_response(
        &self,
        reply: ImageEditReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        self.record(Event::Respond {
            status: reply.raw_response.status_code,
        });
        if self.behavior == Behavior::RefuseResponse {
            return Err(FixtureError::Contract(ImageContractError::NotImplemented {
                message: "the fixture response transform failed",
            }));
        }
        serde_json::from_slice(&reply.raw_response.body).map_err(FixtureError::Reply)
    }

    fn validate_environment(&self, request: ImageEditEnvironment<'_>) -> ImageHeaders {
        self.record(Event::Environment {
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            api_base: request.api_base.map(str::to_owned),
            model: request.model.to_owned(),
            headers: request.headers.clone(),
        });
        headers(&[("x", "validated")])
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        litellm_params: &ImageParams,
    ) -> Result<String, Self::Error> {
        let api_key = match litellm_params.get("api_key") {
            Some(Json::Text(key)) => Some(key.clone()),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => None,
        };
        self.record(Event::Url {
            api_base: api_base.map(str::to_owned),
            api_key,
        });
        if self.behavior == Behavior::RefuseUrl {
            return Err(FixtureError::Contract(ImageContractError::ApiBaseRequired));
        }
        let base = api_base.unwrap_or(&self.base);
        Ok(format!("{base}/images/edits"))
    }

    fn finalize_image_edit_request_data(
        &self,
        mut data: ImageParams,
        resolved_request_url: &str,
    ) -> ImageParams {
        if self.behavior == Behavior::NestFinalized {
            data.insert(
                String::from("nested"),
                Json::Object(IndexMap::from([(String::from("a"), json("1"))])),
            );
        }
        data.insert(String::from("revision"), Json::Text(String::from("final")));
        self.record(Event::Finalize {
            url: resolved_request_url.to_owned(),
            data: data.clone(),
        });
        data
    }

    fn use_multipart_form_data(&self) -> bool {
        self.multipart
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &ImageHeaders,
    ) -> ProviderError {
        self.record(Event::Classify {
            message: error_message.to_owned(),
            status_code,
            headers: headers.clone(),
        });
        base_exception(error_message, status_code, headers)
    }
}

// The config's own refusals, kept apart from the provider's answer
#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error("the image edit contract refused the call")]
    Contract(#[from] ImageContractError),
    #[error("the fixture's reply is not an image response")]
    Reply(#[source] serde_json::Error),
}

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(super) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

// The selected client's wire: it records each request and answers with the one scripted reply or failure
#[derive(Clone, Default)]
pub(super) struct Transport {
    requests: Arc<Mutex<Vec<WriteRequest>>>,
    reply: Arc<Mutex<Option<Reply>>>,
    failure: Arc<Mutex<Option<TransportError>>>,
}

impl Transport {
    pub(super) fn replying(reply: Reply) -> Self {
        let transport = Self::default();
        *transport
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reply);
        transport
    }

    pub(super) fn failing(error: TransportError) -> Self {
        let transport = Self::default();
        *transport
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(error);
        transport
    }

    pub(super) fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WriteTransport for Transport {
    fn default_timeout(&self) -> Duration {
        Duration::default()
    }

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return ready(Err(error));
        }
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match reply {
            Some(reply) => ready(Ok(UpstreamResponse {
                status: reply.status,
                headers: reply.headers,
                body: reply.body,
            })),
            None => ready(Err(TransportError::Closed)),
        }
    }
}

pub(super) struct Source {
    transport: Transport,
    calls: Arc<Mutex<Vec<Option<bool>>>>,
}

impl Source {
    pub(super) fn new(transport: Transport) -> Self {
        Self {
            transport,
            calls: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<Option<bool>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Transport;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Transport, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.transport.clone())
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call a production request logs through, with nothing attached to it
pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    pub(super) state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::requiring("")
    }

    // The same state, but with the raw-request-only flag that turns the pre-call into a refusal
    pub(super) fn requiring_only_raw() -> Self {
        Self::requiring(r#","request_logging":{"raw_request_only":true}"#)
    }

    fn requiring(extra: &str) -> Self {
        let text = format!(
            r#"{{"model":"gpt-image-1","call_type":"image_edit","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"{extra}}}"#
        );
        let init: LoggingInit = serde_json::from_str(&text).expect("the logging fixture is valid");
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

    pub(super) async fn run<C, S>(
        &mut self,
        mode: ClientKind,
        deps: &ImageEditDeps<'_, C, S>,
        call: &ImageEditCall<'_>,
    ) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
    where
        C: BaseImageEditConfig,
        S: ClientSource,
    {
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
            logger_fn: None,
            now: self.now,
        };
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        match mode {
            ClientKind::Sync => image_edit_handler(deps, call, &mut log).await,
            ClientKind::Asynchronous => async_image_edit_handler(deps, call, &mut log).await,
        }
    }

    pub(super) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }

    pub(super) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }
}

pub(super) const fn deps<'deps>(
    config: &'deps RecordingConfig,
    source: &'deps Source,
    supplied: Option<SuppliedClient<'deps, Transport>>,
) -> ImageEditDeps<'deps, RecordingConfig, Source> {
    ImageEditDeps {
        config,
        source,
        supplied,
    }
}

pub(super) fn body_json(request: &WriteRequest) -> Json {
    let RequestBody::Json(body) = &request.body else {
        panic!("the request carries a JSON body");
    };
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(super) const CALL_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) const fn edit_call<'call>(
    prompt: Option<&'call str>,
    image: Option<&'call ImageFile>,
    optional_params: &'call ImageParams,
    litellm_params: &'call ImageParams,
    extra_headers: Option<&'call ImageHeaders>,
) -> ImageEditCall<'call> {
    ImageEditCall {
        model: "gpt-image-1",
        prompt,
        image,
        optional_params,
        litellm_params,
        extra_headers,
        timeout: CALL_TIMEOUT,
    }
}

pub(super) fn provider_error<ConfigError: Error + 'static>(
    error: ImageEditHttpError<ConfigError>,
) -> ProviderError {
    let ImageEditHttpError::Provider(provider) = error else {
        panic!("the call failed outside the provider answer: {error}");
    };
    *provider
}

pub(super) fn request_error<ConfigError: Error + 'static>(
    error: ImageEditHttpError<ConfigError>,
) -> RequestError {
    if let ImageEditHttpError::Request(request) = error {
        return request;
    }
    panic!("the call failed outside the request build: {error}")
}

pub(super) fn pre_post_error<ConfigError: Error + 'static>(
    error: ImageEditHttpError<ConfigError>,
) -> PrePostError {
    match error {
        ImageEditHttpError::Logging(logging) => logging,
        other => panic!("the call failed outside the pre-call logging: {other}"),
    }
}

pub(super) const RESPONSE_BODY: &str = r#"{"created":1,"data":[{"b64_json":"YQ=="}]}"#;
pub(super) const API_BASE: &str = "http://mock.test/v1";
pub(super) const EDITS_URL: &str = "http://mock.test/v1/images/edits";
const MODEL: &str = "gpt-image-1";

pub(super) fn image() -> ImageFile {
    ImageFile {
        filename: Some(String::from("a.png")),
        content: b"abc".to_vec(),
        content_type: Some(String::from("image/png")),
    }
}

pub(super) fn image_response() -> ImageResponse {
    serde_json::from_str(RESPONSE_BODY).expect("the response fixture parses")
}

pub(super) fn litellm_params() -> ImageParams {
    params(r#"{"api_key":"key","api_base":"http://mock.test/v1"}"#)
}

pub(super) fn part(name: &str, data: &[u8]) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: None,
        content_type: None,
        data: data.to_vec(),
    }
}

pub(super) fn file_part(
    name: &str,
    file_name: Option<&str>,
    content_type: Option<&str>,
    data: &[u8],
) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: file_name.map(str::to_owned),
        content_type: content_type.map(str::to_owned),
        data: data.to_vec(),
    }
}

pub(super) fn edit_transport() -> Transport {
    Transport::replying(Reply::json(200, RESPONSE_BODY).with_header("x-request-id", "edit_1"))
}

fn caller_transform_event(mode: ClientKind, image: &ImageFile, options: &ImageParams) -> Event {
    Event::Transform {
        mode,
        model: String::from(MODEL),
        prompt: Some(String::from("change")),
        image: Some(image.clone()),
        options: options.clone(),
        headers: headers(&[("x", "caller")]),
    }
}

pub(super) fn sync_events(image: &ImageFile, options: &ImageParams) -> Vec<Event> {
    vec![
        Event::Environment {
            api_key: Some(String::from("key")),
            api_base: Some(String::from(API_BASE)),
            model: String::from(MODEL),
            headers: headers(&[("x", "option")]),
        },
        Event::Url {
            api_base: Some(String::from(API_BASE)),
            api_key: Some(String::from("key")),
        },
        caller_transform_event(ClientKind::Sync, image, options),
        Event::Finalize {
            url: String::from(EDITS_URL),
            data: params_json(),
        },
        Event::Respond { status: 200 },
    ]
}

pub(super) fn assert_sync_result(outcome: &ImageEditOutcome) {
    assert_eq!(outcome.response, image_response());
    assert_eq!(
        outcome.provider_headers.headers,
        IndexMap::from([(String::from("x-request-id"), String::from("edit_1"))])
    );
    assert_eq!(
        outcome.provider_headers.additional_headers,
        IndexMap::from([(
            String::from("llm_provider-x-request-id"),
            Json::Text(String::from("edit_1")),
        )])
    );
}

pub(super) fn assert_sync_request(transport: &Transport) {
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url.as_str(), EDITS_URL);
    assert_eq!(request.timeout, Some(CALL_TIMEOUT));
    assert_eq!(
        request
            .headers
            .get("x")
            .and_then(|value| value.to_str().ok()),
        Some("caller")
    );
    let RequestBody::Multipart { parts, .. } = &request.body else {
        panic!("the fixture config asks for a multipart body");
    };
    assert_eq!(
        parts,
        &vec![
            part("prompt", b"change"),
            part("revision", b"final"),
            file_part("image", Some("a.png"), Some("image/png"), b"abc"),
        ]
    );
    assert!(parts.iter().all(|part| part.name != "none"));
}

pub(super) fn assert_sync_logging(fixture: &Fixture) {
    assert_eq!(fixture.input(), Some(&Json::Text(String::from("change"))));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(EDITS_URL)))
    );
    assert_eq!(fixture.logged("headers"), Some(&json(r#"{"x":"caller"}"#)));
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(
            r#"{"image":{"filename":"a.png","content":"abc","content_type":"image/png"}}"#
        ))
    );
    assert!(fixture.state.model_call_details.get("api_key").is_none());
}

pub(super) async fn assert_async_matches_sync(
    deps: &ImageEditDeps<'_, RecordingConfig, Source>,
    call: &ImageEditCall<'_>,
    transport: &Transport,
    image: &ImageFile,
    options: &ImageParams,
) {
    let mut fixture = Fixture::new();
    let outcome = fixture
        .run(ClientKind::Asynchronous, deps, call)
        .await
        .expect("the async edit succeeds");
    assert_eq!(outcome.response, image_response());
    let transformed = caller_transform_event(ClientKind::Asynchronous, image, options);
    assert!(deps.config.events().contains(&transformed));
    assert_eq!(fixture.logged("complete_input_dict"), Some(&params_json()));
    assert!(fixture.state.model_call_details.get("api_key").is_none());
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let RequestBody::Multipart { parts, .. } = &requests[0].body else {
        panic!("the async path sends the same multipart body");
    };
    assert_eq!(parts.len(), 3);
}

pub(super) fn params_json() -> Json {
    json(r#"{"prompt":"change","revision":"final"}"#)
}

pub(super) fn validated_transform_event(mode: ClientKind) -> Event {
    Event::Transform {
        mode,
        model: String::from(MODEL),
        prompt: Some(String::from("change")),
        image: None,
        options: ImageParams::new(),
        headers: headers(&[("x", "validated")]),
    }
}

pub(super) fn validated_events() -> Vec<Event> {
    vec![
        Event::Environment {
            api_key: Some(String::from("key")),
            api_base: Some(String::from(API_BASE)),
            model: String::from(MODEL),
            headers: ImageHeaders::new(),
        },
        Event::Url {
            api_base: Some(String::from(API_BASE)),
            api_key: Some(String::from("key")),
        },
        validated_transform_event(ClientKind::Sync),
        Event::Finalize {
            url: String::from(EDITS_URL),
            data: params_json(),
        },
        Event::Respond { status: 200 },
    ]
}

pub(super) fn assert_async_transform_once(config: &RecordingConfig) {
    let events = config.events();
    let transforms: Vec<&Event> = events
        .iter()
        .filter(|event| matches!(event, Event::Transform { .. }))
        .collect();
    assert_eq!(transforms.len(), 1);
    assert!(matches!(
        transforms[0],
        Event::Transform {
            mode: ClientKind::Asynchronous,
            ..
        }
    ));
}

fn assert_field_error(
    fixture: &mut Fixture,
    deps: &ImageEditDeps<'_, RecordingConfig, Source>,
    optional: &ImageParams,
    litellm: &ImageParams,
    expected: &str,
) {
    let call = edit_call(Some("change"), None, optional, litellm, None);
    let Err(error) = fixture.run(ClientKind::Sync, deps, &call).await else {
        panic!("the wrong-typed {expected} fails the call");
    };
    let RequestError::FieldType { field } = request_error(error) else {
        panic!("the wrong-typed {expected} is a field type");
    };
    assert_eq!(field, expected);
}

pub(super) async fn assert_validation_never_selects(
    fixture: &mut Fixture,
    deps: &ImageEditDeps<'_, RecordingConfig, Source>,
    good: &ImageParams,
) {
    let options = ImageParams::new();
    let bad_key = params(r#"{"api_key":7,"api_base":"http://mock.test/v1"}"#);
    assert_field_error(fixture, deps, &options, &bad_key, "litellm_params.api_key").await;
    let bad_verify = params(r#"{"ssl_verify":"yes","api_base":"http://mock.test/v1"}"#);
    assert_field_error(
        fixture,
        deps,
        &options,
        &bad_verify,
        "litellm_params.ssl_verify",
    )
    .await;
    let bad_base = params(r#"{"api_base":["x"]}"#);
    assert_field_error(
        fixture,
        deps,
        &options,
        &bad_base,
        "litellm_params.api_base",
    )
    .await;
    let bad_options = params(r#"{"extra_headers":{"x":1}}"#);
    assert_field_error(
        fixture,
        deps,
        &bad_options,
        good,
        "optional_params.extra_headers",
    )
    .await;
}
