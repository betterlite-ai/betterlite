#![cfg(test)]

use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::video_records::VideoObject;
use crate::api::video_status_transport::clients::{AsyncClient, ClientFactories, SyncClient};
use crate::api::video_status_transport::config::{VideoErrorClasses, VideoStatusConfig};
use crate::api::video_status_transport::error::{
    BaseLlmException, ErrorConverter, ProviderFailure, StatusFailure,
};
use crate::api::video_status_transport::records::{
    Headers, Params, StatusRequest, VideoEnvironment, VideoReply, VideoRequestContext,
};

pub(crate) const VIDEO_ID: &str = "video_1";
pub(crate) const PROVIDER: &str = "openai";
pub(crate) const REPLY_HEADER: &str = "x-provider-reply";
const REPLY_HEADER_VALUE: &str = "yes";
pub(crate) const API_BASE: &str = "https://given/v1";
pub(crate) const COMPLETE_URL: &str = "https://built/videos/video_1";
pub(crate) const PROBE_STATUS: u16 = 599;
pub(crate) const OK: FakeReply = FakeReply {
    status: 200,
    body: "video_1",
    fail: false,
};
pub(crate) const FAILING: FakeReply = FakeReply {
    status: 200,
    body: "",
    fail: true,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FakeReply {
    pub(crate) status: u16,
    pub(crate) body: &'static str,
    pub(crate) fail: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Supplied,
    Built,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Supply {
    None,
    Sync,
    Async,
}

// the two response forms, whose fakes differ only in the journal list and marker they use
#[derive(Clone, Copy, Debug)]
enum Form {
    Sync,
    Async,
}

// what B249's logger fn gets from pre_call: the input and the additional args it recorded
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedPreCall {
    pub(crate) input: Json,
    pub(crate) additional_args: Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    ValidateEnvironment,
    CompleteUrl,
    RequestTransform,
    PreCall,
    SyncBuild,
    AsyncBuild,
    SyncSend,
    AsyncSend,
    SyncResponse,
    AsyncResponse,
    ErrorClass,
    Converted,
}

// One call's observations in order, with each kind's payload kept beside the sequence
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Journal {
    pub(crate) kinds: Vec<Kind>,
    pub(crate) validations: Vec<Validation>,
    pub(crate) urls: Vec<UrlRecord>,
    pub(crate) transforms: Vec<TransformRecord>,
    pub(crate) pre_calls: Vec<RecordedPreCall>,
    pub(crate) sync_builds: Vec<Option<bool>>,
    pub(crate) async_builds: Vec<(ProviderId, Option<bool>)>,
    pub(crate) sync_sends: Vec<(Owner, StatusRequest)>,
    pub(crate) async_sends: Vec<(Owner, StatusRequest)>,
    pub(crate) sync_responses: Vec<ResponseRecord>,
    pub(crate) async_responses: Vec<ResponseRecord>,
    pub(crate) error_classes: Vec<(String, u16)>,
    pub(crate) conversions: Vec<String>,
}

pub(crate) type Recorder = Arc<Mutex<Journal>>;

pub(crate) fn recorder() -> Recorder {
    Arc::new(Mutex::new(Journal::default()))
}

pub(crate) fn record<Update>(events: &Recorder, kind: Kind, update: Update)
where
    Update: FnOnce(&mut Journal),
{
    let mut journal = events.lock().unwrap_or_else(PoisonError::into_inner);
    journal.kinds.push(kind);
    update(&mut journal);
}

pub(crate) fn observed(events: &Recorder) -> Journal {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(crate) fn object(id: &str) -> VideoObject {
    VideoObject::new(id.to_owned(), "queued".to_owned())
}

// the transform's own data map, as Vertex AI's fetchPredictOperation body
pub(crate) fn operation() -> Params {
    Params::from([(
        "operationName".to_owned(),
        Json::Text("projects/p/operations/o".to_owned()),
    )])
}

pub(crate) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) struct FakeConfig {
    pub(crate) events: Recorder,
    pub(crate) environment: Result<Headers, ProviderFailure>,
    pub(crate) complete_url: Result<String, ProviderFailure>,
    pub(crate) transformed: Result<(String, Params), ProviderFailure>,
    pub(crate) response: Result<VideoObject, ProviderFailure>,
    pub(crate) asynchronous_response: Result<VideoObject, ProviderFailure>,
}

impl VideoErrorClasses for FakeConfig {
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> BaseLlmException {
        record(&self.events, Kind::ErrorClass, |journal| {
            journal
                .error_classes
                .push((error_message.to_owned(), status_code));
        });
        BaseLlmException {
            status_code,
            message: format!("fake error class: {error_message}"),
            headers: headers.clone(),
            status_code_is_synthesized: false,
        }
    }
}

#[async_trait]
impl VideoStatusConfig for FakeConfig {
    type SyncClient = FakeSyncClient;
    type AsyncClient = FakeAsyncClient;

    fn validate_environment(
        &self,
        request: VideoEnvironment<'_>,
    ) -> Result<Headers, ProviderFailure> {
        record(&self.events, Kind::ValidateEnvironment, |journal| {
            journal.validations.push(Validation {
                headers: request.headers.clone(),
                model: request.model.to_owned(),
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                params: request.litellm_params.cloned(),
            });
        });
        self.environment.clone().map(|provider| {
            let mut headers = request.headers.clone();
            headers.extend(provider);
            headers
        })
    }

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, ProviderFailure> {
        record(&self.events, Kind::CompleteUrl, |journal| {
            journal.urls.push(UrlRecord {
                model: model.to_owned(),
                api_base: api_base.map(ToOwned::to_owned),
                params: litellm_params.clone(),
            });
        });
        self.complete_url.clone()
    }

    fn transform_video_status_retrieve_request(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ProviderFailure> {
        record(&self.events, Kind::RequestTransform, |journal| {
            journal.transforms.push(TransformRecord {
                video_id: video_id.to_owned(),
                api_base: context.api_base.to_owned(),
                params: context.litellm_params.clone(),
                headers: context.headers.clone(),
            });
        });
        self.transformed.clone()
    }

    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&FakeSyncClient>,
    ) -> Result<VideoObject, ProviderFailure> {
        self.finish(reply, client.map(|used| used.owner), Form::Sync)
    }

    async fn async_transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&FakeAsyncClient>,
    ) -> Result<VideoObject, ProviderFailure> {
        self.finish(reply, client.map(|used| used.owner), Form::Async)
    }
}

impl FakeConfig {
    fn finish(
        &self,
        reply: VideoReply<'_>,
        owner: Option<Owner>,
        form: Form,
    ) -> Result<VideoObject, ProviderFailure> {
        let recorded = ResponseRecord {
            owner,
            provider: reply.custom_llm_provider.map(ToOwned::to_owned),
            status: reply.raw_response.status,
            body: reply.raw_response.body.clone(),
            header: reply_header(reply.raw_response),
        };
        let (mark, outcome) = match form {
            Form::Sync => ("sync_transform", self.response.clone()),
            Form::Async => ("async_transform", self.asynchronous_response.clone()),
        };
        match form {
            Form::Sync => record(&self.events, Kind::SyncResponse, |journal| {
                journal.sync_responses.push(recorded);
            }),
            Form::Async => record(&self.events, Kind::AsyncResponse, |journal| {
                journal.async_responses.push(recorded);
            }),
        }
        reply
            .logging_obj
            .model_call_details
            .insert(mark.to_owned(), Json::Bool(true));
        outcome
    }
}

pub(crate) struct FakeFactories {
    pub(crate) events: Recorder,
    pub(crate) sync: FakeReply,
    pub(crate) asynchronous: FakeReply,
    pub(crate) refuse_sync: bool,
    pub(crate) refuse_async: bool,
}

impl ClientFactories for FakeFactories {
    type Synchronous = FakeSyncClient;
    type Asynchronous = FakeAsyncClient;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::Synchronous, PoolError> {
        self.sync_or_closed(ssl_verify)
    }

    fn async_client(
        &self,
        provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Self::Asynchronous, PoolError> {
        self.async_or_closed(provider, ssl_verify)
    }
}

impl FakeFactories {
    fn sync_or_closed(&self, ssl_verify: Option<bool>) -> Result<FakeSyncClient, PoolError> {
        record(&self.events, Kind::SyncBuild, |journal| {
            journal.sync_builds.push(ssl_verify);
        });
        allowed(self.refuse_sync, self.sync_built())
    }

    fn async_or_closed(
        &self,
        provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<FakeAsyncClient, PoolError> {
        record(&self.events, Kind::AsyncBuild, |journal| {
            journal.async_builds.push((provider, ssl_verify));
        });
        allowed(self.refuse_async, self.async_built())
    }

    fn sync_built(&self) -> FakeSyncClient {
        FakeSyncClient {
            events: Arc::clone(&self.events),
            owner: Owner::Built,
            reply: self.sync,
        }
    }

    fn async_built(&self) -> FakeAsyncClient {
        FakeAsyncClient {
            events: Arc::clone(&self.events),
            owner: Owner::Built,
            reply: self.asynchronous,
        }
    }
}

fn allowed<T>(refuse: bool, value: T) -> Result<T, PoolError> {
    if refuse {
        return Err(PoolError::Closed);
    }
    Ok(value)
}

pub(crate) struct FakeSyncClient {
    pub(crate) events: Recorder,
    pub(crate) owner: Owner,
    pub(crate) reply: FakeReply,
}

fn send_with(
    events: &Recorder,
    owner: Owner,
    request: &StatusRequest,
    form: Form,
    reply: FakeReply,
) -> Result<UpstreamResponse, TransportError> {
    let sent = request.clone();
    match form {
        Form::Sync => record(events, Kind::SyncSend, |journal| {
            journal.sync_sends.push((owner, sent));
        }),
        Form::Async => record(events, Kind::AsyncSend, |journal| {
            journal.async_sends.push((owner, sent));
        }),
    }
    answer(reply)
}

impl SyncClient for FakeSyncClient {
    fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError> {
        send_with(&self.events, self.owner, request, Form::Sync, self.reply)
    }
}

pub(crate) struct FakeAsyncClient {
    pub(crate) events: Recorder,
    pub(crate) owner: Owner,
    pub(crate) reply: FakeReply,
}

#[async_trait]
impl AsyncClient for FakeAsyncClient {
    async fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError> {
        send_with(&self.events, self.owner, request, Form::Async, self.reply)
    }
}

pub(crate) fn answer(reply: FakeReply) -> Result<UpstreamResponse, TransportError> {
    if reply.fail {
        return Err(TransportError::Closed);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(REPLY_HEADER),
        HeaderValue::from_static(REPLY_HEADER_VALUE),
    );
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.as_bytes().to_vec(),
    })
}

// what a provider's response transform reads back out of the raw reply
fn reply_header(response: &UpstreamResponse) -> Option<String> {
    response
        .headers
        .get(REPLY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// B249's logging environment, with a logger fn that records what pre_call handed it
pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    logger: Box<dyn Fn(&Json) -> Result<(), PrePostError> + Sync>,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(events: &Recorder) -> Self {
        let events = Arc::clone(events);
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor compiles"),
            sink: Sink,
            logger: Box::new(move |details: &Json| -> Result<(), PrePostError> {
                let recorded = recorded_pre_call(details);
                record(&events, Kind::PreCall, |journal| {
                    journal.pre_calls.push(recorded);
                });
                Ok(())
            }),
            now: moment(),
        }
    }

    pub(crate) fn environment(&self) -> PrePostEnv<'_> {
        PrePostEnv {
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
            logger_fn: Some(&*self.logger),
            now: self.now,
        }
    }
}

// pre_call records these before it calls the logger fn, which is what the fake above captures
pub(crate) fn recorded_pre_call(details: &Json) -> RecordedPreCall {
    let fields = match details {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("pre_call hands its logger fn an object")
        }
    };
    RecordedPreCall {
        input: fields.get("input").cloned().unwrap_or(Json::Null),
        additional_args: fields.get("additional_args").cloned().unwrap_or(Json::Null),
    }
}

pub(crate) struct FakeConverter {
    pub(crate) events: Recorder,
}

impl ErrorConverter for FakeConverter {
    fn convert(
        &self,
        config: &dyn VideoErrorClasses,
        failure: StatusFailure<'_>,
    ) -> BaseLlmException {
        let caught = match failure {
            StatusFailure::Send(_) => "send".to_owned(),
            StatusFailure::Transform(error) => format!("transform: {}", error.message),
        };
        record(&self.events, Kind::Converted, |journal| {
            journal.conversions.push(caught);
        });
        let probe = config.get_error_class("probe", PROBE_STATUS, &Headers::new());
        BaseLlmException {
            status_code: probe.status_code,
            message: probe.message,
            headers: probe.headers,
            status_code_is_synthesized: true,
        }
    }
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

// a caller header a provider default never sets, so the merge shows in the recorded args
pub(crate) fn plain_headers() -> Headers {
    Headers::from([("x-caller".to_owned(), "given".to_owned())])
}

// the provider's Authorization overwrites the caller's, as LiteLLM's in-place update does, so the merge test uses this one
pub(crate) fn extra_headers() -> Headers {
    Headers::from([
        ("x-caller".to_owned(), "given".to_owned()),
        ("authorization".to_owned(), "caller".to_owned()),
    ])
}

pub(crate) const fn duration() -> Duration {
    Duration::from_secs(9)
}

pub(crate) const fn reply(status: u16, body: &'static str) -> FakeReply {
    FakeReply {
        status,
        body,
        fail: false,
    }
}

pub(crate) fn provider_failure(message: &str) -> ProviderFailure {
    ProviderFailure {
        message: message.to_owned(),
        status_code: None,
        headers: None,
    }
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(crate) fn ssl_params(verify: bool) -> Params {
    Params::from([("ssl_verify".to_owned(), Json::Bool(verify))])
}

pub(crate) fn text_ssl_params() -> Params {
    Params::from([("ssl_verify".to_owned(), Json::Text("yes".to_owned()))])
}

pub(crate) fn base_params(base: &str) -> Params {
    Params::from([("api_base".to_owned(), Json::Text(base.to_owned()))])
}

pub(crate) fn number_base_params() -> Params {
    Params::from([("api_base".to_owned(), json("7"))])
}

// a provider default the caller's extra headers must override
pub(crate) fn provider_headers() -> Headers {
    Headers::from([
        ("authorization".to_owned(), "provider".to_owned()),
        ("x-provider".to_owned(), "default".to_owned()),
    ])
}

// what the request transform sees once the provider's validation has overwritten the caller's headers
pub(crate) fn merged_headers() -> Headers {
    Headers::from([
        ("authorization".to_owned(), "provider".to_owned()),
        ("x-provider".to_owned(), "default".to_owned()),
        ("x-caller".to_owned(), "given".to_owned()),
    ])
}

pub(crate) fn provider() -> ProviderId {
    ProviderId::from_str(PROVIDER).expect("the fixture provider is known")
}

// a second known provider, so a test proves the resolved name reaches the factory and the transform
pub(crate) fn runway() -> ProviderId {
    ProviderId::from_str("runwayml").expect("the fixture provider is known")
}

// an extra_body the request must never carry
pub(crate) fn unused_body() -> Params {
    Params::from([("ignored".to_owned(), Json::Bool(true))])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Validation {
    pub(crate) headers: Headers,
    pub(crate) model: String,
    pub(crate) api_key: Option<String>,
    pub(crate) params: Option<Params>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UrlRecord {
    pub(crate) model: String,
    pub(crate) api_base: Option<String>,
    pub(crate) params: Params,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TransformRecord {
    pub(crate) video_id: String,
    pub(crate) api_base: String,
    pub(crate) params: Params,
    pub(crate) headers: Headers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResponseRecord {
    pub(crate) owner: Option<Owner>,
    pub(crate) provider: Option<String>,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) header: Option<String>,
}

pub(crate) fn validate_calls(journal: &Journal) -> Vec<Validation> {
    journal.validations.clone()
}

pub(crate) fn complete_urls(journal: &Journal) -> Vec<UrlRecord> {
    journal.urls.clone()
}

pub(crate) fn request_transforms(journal: &Journal) -> Vec<TransformRecord> {
    journal.transforms.clone()
}

pub(crate) fn pre_calls(journal: &Journal) -> Vec<RecordedPreCall> {
    journal.pre_calls.clone()
}

pub(crate) fn sync_builds(journal: &Journal) -> Vec<Option<bool>> {
    journal.sync_builds.clone()
}

pub(crate) fn async_builds(journal: &Journal) -> Vec<(ProviderId, Option<bool>)> {
    journal.async_builds.clone()
}

pub(crate) fn sync_sends(journal: &Journal) -> Vec<(Owner, StatusRequest)> {
    journal.sync_sends.clone()
}

pub(crate) fn async_sends(journal: &Journal) -> Vec<(Owner, StatusRequest)> {
    journal.async_sends.clone()
}

pub(crate) fn sync_responses(journal: &Journal) -> Vec<ResponseRecord> {
    journal.sync_responses.clone()
}

pub(crate) fn async_responses(journal: &Journal) -> Vec<ResponseRecord> {
    journal.async_responses.clone()
}

pub(crate) fn error_classes(journal: &Journal) -> Vec<(String, u16)> {
    journal.error_classes.clone()
}

pub(crate) fn conversions(journal: &Journal) -> Vec<String> {
    journal.conversions.clone()
}
