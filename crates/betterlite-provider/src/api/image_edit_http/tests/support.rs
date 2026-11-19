use std::error::Error;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::UpstreamResponse;
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
    ClientKind, ClientSource, EditRequest, EditTransport, ImageEditCall, ImageEditDeps,
    ImageEditOutcome, LogContext, SuppliedClient,
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
    requests: Arc<Mutex<Vec<EditRequest>>>,
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

    pub(super) fn requests(&self) -> MutexGuard<'_, Vec<EditRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl EditTransport for Transport {
    fn send(
        &self,
        request: EditRequest,
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

pub(super) fn body_json(request: &EditRequest) -> Json {
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
    match error {
        ImageEditHttpError::Provider(error) => *error,
        other => panic!("the call failed outside the provider answer: {other}"),
    }
}

pub(super) fn request_error<ConfigError: Error + 'static>(
    error: ImageEditHttpError<ConfigError>,
) -> RequestError {
    match error {
        ImageEditHttpError::Request(error) => error,
        other => panic!("the call failed outside the request build: {other}"),
    }
}

pub(super) fn pre_post_error<ConfigError: Error + 'static>(
    error: ImageEditHttpError<ConfigError>,
) -> PrePostError {
    match error {
        ImageEditHttpError::Logging(error) => error,
        other => panic!("the call failed outside the pre-call logging: {other}"),
    }
}
