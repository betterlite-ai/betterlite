use std::future::{Future, ready};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use crate::api::video_edit_transport::calls::{Handled, VideoExtensionRequest};
use crate::api::video_edit_transport::config::{
    BaseVideoConfig, Headers, Params, RequestFiles, VideoEditInput, VideoEnvironment,
    VideoExtensionInput, VideoReply, VideoRequestContext,
};
use crate::api::video_edit_transport::error::{BoxError, FieldTypeError, VideoEditTransportError};
use crate::api::video_edit_transport::extensions::VideoExtensionHandlers;
use crate::api::video_extension_call::{
    VideoExtensionCall, VideoExtensionConfigLookup, VideoExtensionDeps,
};
use crate::api::video_records::VideoObject;
use crate::api::video_selection::VideoConfigTag;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const INITIAL_CALL_TYPE: &str = "endpoint_func";
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(6_000);

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "video".to_owned(),
            call_type: INITIAL_CALL_TYPE.to_owned(),
            start_time: moment(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(crate) fn video(id: &str) -> VideoObject {
    VideoObject::new(id.to_owned(), "completed".to_owned())
}

// the loose keywords a real call carries: the SDK wrapper puts the logging object there
pub(crate) fn base_kwargs() -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    kwargs.insert("litellm_logging_obj".to_owned(), Json::Bool(true));
    kwargs
}

pub(crate) fn provider_error(status_code: u16, message: String) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message,
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}

pub(crate) fn handler_error(status_code: u16, message: String) -> VideoEditTransportError {
    VideoEditTransportError::Provider(Box::new(provider_error(status_code, message)))
}

pub(crate) const fn broken_error() -> VideoEditTransportError {
    VideoEditTransportError::FieldType(FieldTypeError {
        field: "ssl_verify",
        expected: "a boolean",
    })
}

pub(crate) struct FakeConfig;

#[rustfmt::skip]
impl BaseVideoConfig for FakeConfig {
    fn validate_environment(&self, _request: VideoEnvironment<'_>) -> Headers { Headers::new() }
    fn get_complete_url(&self, _model: &str, _api_base: Option<&str>, _litellm_params: &Params) -> Result<String, BoxError> { Ok(String::new()) }
    fn get_video_edit_prefetch_params(&self, _video_id: &str, _context: VideoRequestContext<'_>) -> Option<(String, Params)> { None }
    fn transform_video_edit_request(&self, _input: VideoEditInput<'_>, _context: VideoRequestContext<'_>) -> Result<(String, Params, Option<RequestFiles>), BoxError> { Ok((String::new(), Params::new(), None)) }
    fn transform_video_edit_response(&self, _reply: VideoReply<'_>, _request_data: Option<&Params>) -> Result<VideoObject, BoxError> { Ok(video("video_0")) }
    fn transform_video_extension_request(&self, _input: VideoExtensionInput<'_>, _context: VideoRequestContext<'_>) -> Result<(String, Params), BoxError> { Ok((String::new(), Params::new())) }
    fn transform_video_extension_response(&self, _reply: VideoReply<'_>) -> Result<VideoObject, BoxError> { Ok(video("video_0")) }
    fn get_error_class(&self, error_message: &str, status_code: u16, _headers: &Headers) -> ProviderError { provider_error(status_code, error_message.to_owned()) }
}

// the caller's built configs, recording every tag B3332 selects and answering for the tags a test lists
pub(crate) struct FakeConfigs {
    config: FakeConfig,
    known: Vec<VideoConfigTag>,
    asked: Mutex<Vec<VideoConfigTag>>,
}

impl FakeConfigs {
    pub(crate) fn new(known: &[VideoConfigTag]) -> Self {
        Self {
            config: FakeConfig,
            known: known.to_vec(),
            asked: Mutex::default(),
        }
    }

    pub(crate) fn asked(&self) -> Vec<VideoConfigTag> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl VideoExtensionConfigLookup for FakeConfigs {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn BaseVideoConfig> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tag.clone());
        let config: &dyn BaseVideoConfig = &self.config;
        self.known.contains(tag).then_some(config)
    }
}

#[derive(Clone)]
pub(crate) enum Reply {
    Done(VideoObject),
    Pending(VideoObject),
    Failed { status_code: u16, message: String },
    Broken,
}

pub(crate) struct FakeClient {
    pub(crate) name: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RecordedExtension {
    pub(crate) prompt: String,
    pub(crate) video_id: String,
    pub(crate) seconds: String,
    pub(crate) provider: String,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) client: Option<(ClientKind, &'static str)>,
    pub(crate) api_key_present: bool,
    pub(crate) is_async: bool,
}

impl RecordedExtension {
    fn of(request: &VideoExtensionRequest<'_, FakeClient>) -> Self {
        Self {
            prompt: request.prompt.to_owned(),
            video_id: request.video_id.to_owned(),
            seconds: request.seconds.to_owned(),
            provider: request.provider.to_owned(),
            litellm_params: request.litellm_params.clone(),
            extra_headers: request.extra_headers.cloned(),
            extra_body: request.extra_body.cloned(),
            timeout: request.timeout,
            client: request
                .client
                .as_ref()
                .map(|client| (client.kind, client.transport.name)),
            api_key_present: request.api_key.is_some(),
            is_async: request.is_async,
        }
    }
}

pub(crate) struct FakeHandler {
    calls: Mutex<Vec<RecordedExtension>>,
    reply: Reply,
}

impl FakeHandler {
    pub(crate) fn new(reply: Reply) -> Self {
        Self {
            calls: Mutex::default(),
            reply,
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedExtension> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &VideoExtensionRequest<'_, FakeClient>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedExtension::of(request));
    }
}

fn reply_result(reply: &Reply) -> Result<VideoObject, VideoEditTransportError> {
    match reply {
        Reply::Done(object) | Reply::Pending(object) => Ok(object.clone()),
        Reply::Failed {
            status_code,
            message,
        } => Err(handler_error(*status_code, message.clone())),
        Reply::Broken => Err(broken_error()),
    }
}

impl VideoExtensionHandlers for FakeHandler {
    type Client = FakeClient;

    fn video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, FakeClient>,
        _config: &'call dyn BaseVideoConfig,
    ) -> Result<Handled<'call>, VideoEditTransportError> {
        self.record(&request);
        match self.reply.clone() {
            Reply::Pending(object) => Ok(Handled::Pending(Box::pin(ready(Ok(object))))),
            Reply::Done(_) | Reply::Failed { .. } | Reply::Broken => {
                reply_result(&self.reply).map(|object| Handled::Done(Box::new(object)))
            }
        }
    }

    fn async_video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, FakeClient>,
        _config: &'call dyn BaseVideoConfig,
    ) -> impl Future<Output = Result<VideoObject, VideoEditTransportError>> + Send + 'call {
        self.record(&request);
        let result = reply_result(&self.reply);
        async move { result }
    }
}

pub(crate) struct Rig {
    pub(crate) handler: FakeHandler,
    pub(crate) configs: FakeConfigs,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
    pub(crate) client: FakeClient,
    pub(crate) request_timeout: Duration,
}

impl Rig {
    pub(crate) fn new(reply: Reply, known: &[VideoConfigTag]) -> Self {
        Self {
            handler: FakeHandler::new(reply),
            configs: FakeConfigs::new(known),
            logging: Mutex::new(logging()),
            redactor: redactor(),
            client: FakeClient { name: "supplied" },
            request_timeout: REQUEST_TIMEOUT,
        }
    }

    pub(crate) fn deps(
        &self,
        client: Option<ClientKind>,
    ) -> VideoExtensionDeps<'_, FakeConfigs, FakeHandler> {
        VideoExtensionDeps {
            configs: &self.configs,
            handler: &self.handler,
            logging: &self.logging,
            redactor: &self.redactor,
            request_timeout: self.request_timeout,
            client: client.map(|kind| SuppliedClient {
                kind,
                transport: &self.client,
            }),
        }
    }
}

pub(crate) fn locked(rig: &Rig) -> MutexGuard<'_, LoggingState> {
    rig.logging.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) const fn call<'call>(
    video_id: &'call str,
    prompt: &'call str,
    seconds: &'call str,
    kwargs: &'call IndexMap<String, Json>,
) -> VideoExtensionCall<'call> {
    VideoExtensionCall {
        video_id,
        prompt,
        seconds,
        timeout: None,
        custom_llm_provider: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        kwargs,
    }
}

pub(crate) fn done(answer: Handled<'_>) -> VideoObject {
    let Handled::Done(object) = answer else {
        panic!("the call answers with the object");
    };
    *object
}

pub(crate) fn only_call(rig: &Rig) -> RecordedExtension {
    let calls = rig.handler.calls();
    assert_eq!(calls.len(), 1, "the handler answers exactly one call");
    calls.into_iter().next().expect("one recorded call")
}

pub(crate) fn connection(class: &ErrorClass) -> &ErrorContext {
    let ErrorClass::ApiConnection(context) = class else {
        panic!("the failure maps to APIConnectionError, got {class:?}");
    };
    context
}

pub(crate) fn internal(class: &ErrorClass) -> &ErrorContext {
    let ErrorClass::InternalServer(context) = class else {
        panic!("the failure maps to InternalServerError, got {class:?}");
    };
    context
}
