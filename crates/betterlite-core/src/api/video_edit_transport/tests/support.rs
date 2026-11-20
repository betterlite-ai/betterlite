use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http_fixture::error::FixtureError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use http::HeaderValue;
use http::header::HeaderMap as HttpHeaders;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::runtime::Builder;

use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::vector_store_http_write::service::{
    ClientKind, SuppliedClient, WriteRequest, WriteTransport,
};
use crate::api::video_edit_transport::calls::{Handled, VideoEditRequest, VideoExtensionRequest};
use crate::api::video_edit_transport::config::{
    BaseVideoConfig, Headers, Params, RequestFiles, VideoEditInput, VideoEnvironment,
    VideoExtensionInput, VideoLogging, VideoReply, VideoRequestContext,
};
use crate::api::video_edit_transport::error::{
    BoxError, VideoClientError, VideoEditTransportError,
};
use crate::api::video_edit_transport::handler::VideoTransport;
use crate::api::video_edit_transport::seams::{
    SharedErrorConvert, VideoClientSource, VideoLoggingHandle, VideoPreCall,
};
use crate::api::video_records::VideoObject;

pub(crate) const KNOWN_PROVIDER: &str = "openai";

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct Refusal {
    pub(crate) message: String,
}

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

// One canned upstream answer, taken in order: the prefetch reply first when a call sends two requests
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
            HeaderValue::from_str(value).expect("the fixture header value"),
        );
        self
    }
}

// The wire every case without a real server talks to: it answers with the queued replies and records each request
#[derive(Clone)]
pub(crate) struct Wire {
    replies: Arc<Mutex<Vec<Reply>>>,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<WriteRequest>>>,
    events: Events,
    default_timeout: Duration,
}

impl Wire {
    pub(crate) fn new(replies: Vec<Reply>, events: &Events) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies)),
            failure: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
            default_timeout: Duration::from_secs(60),
        }
    }

    pub(crate) fn failing(error: TransportError, events: &Events) -> Self {
        let wire = Self::new(Vec::new(), events);
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WriteTransport for Wire {
    fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    async fn send(&self, request: WriteRequest) -> Result<UpstreamResponse, TransportError> {
        self.events.push("send");
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(error) = failure {
            return ready(Err(error));
        }
        let reply = reply.expect("the wire has a reply left");
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: reply.headers,
            body: reply.body,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceCall {
    pub(crate) mode: ClientKind,
    pub(crate) provider: String,
    pub(crate) ssl_verify: Option<bool>,
}

// B173's pooled client factories: a supplied client never reaches them, and an unknown provider fails here
pub(crate) struct FakeSource<Client> {
    client: Client,
    calls: Arc<Mutex<Vec<SourceCall>>>,
}

impl<Client> FakeSource<Client> {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<SourceCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Client: WriteTransport + Clone> VideoClientSource for FakeSource<Client> {
    type Client = Client;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Client, VideoClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SourceCall {
                mode: ClientKind::Sync,
                provider: String::new(),
                ssl_verify,
            });
        Ok(self.client.clone())
    }

    fn async_client(
        &self,
        provider: &str,
        ssl_verify: Option<bool>,
    ) -> Result<Client, VideoClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SourceCall {
                mode: ClientKind::Asynchronous,
                provider: provider.to_owned(),
                ssl_verify,
            });
        if provider == KNOWN_PROVIDER {
            return Ok(self.client.clone());
        }
        Err(VideoClientError::UnknownProvider {
            provider: provider.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedPreCall {
    pub(crate) input: String,
    pub(crate) api_key: String,
    pub(crate) complete_input_dict: Params,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
    pub(crate) video_id: String,
}

// B249's logging handle: it records each pre_call and answers the response transform with the same logging object
pub(crate) struct FakeLog {
    pub(crate) events: Events,
    pub(crate) calls: Arc<Mutex<Vec<RecordedPreCall>>>,
    pub(crate) object: Params,
    pub(crate) refusal: Option<String>,
}

impl FakeLog {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
            calls: Arc::default(),
            object: Params::new(),
            refusal: None,
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedPreCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl VideoLoggingHandle for FakeLog {
    fn pre_call(&self, call: &VideoPreCall<'_>) -> Result<(), PrePostError> {
        self.events.push("pre-call");
        if let Some(message) = &self.refusal {
            return Err(PrePostError::CallbackFailed {
                message: message.clone(),
            });
        }
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedPreCall {
                input: call.input.to_owned(),
                api_key: call.api_key.to_owned(),
                complete_input_dict: call.complete_input_dict.clone(),
                api_base: call.api_base.to_owned(),
                headers: call.headers.clone(),
                video_id: call.video_id.to_owned(),
            });
        Ok(())
    }

    fn logging_object(&self) -> VideoLogging {
        VideoLogging {
            model_call_details: self.object.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Validated {
    pub(crate) headers: Headers,
    pub(crate) model: String,
    pub(crate) api_key: Option<String>,
    pub(crate) params: Option<Params>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlSeen {
    pub(crate) model: String,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrefetchSeen {
    pub(crate) video_id: String,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditSeen {
    pub(crate) prompt: String,
    pub(crate) video_id: String,
    pub(crate) video_file: bool,
    pub(crate) extra_body: Option<Params>,
    pub(crate) prefetched: Option<Params>,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtensionSeen {
    pub(crate) prompt: String,
    pub(crate) video_id: String,
    pub(crate) seconds: String,
    pub(crate) extra_body: Option<Params>,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplySeen {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) provider: Option<String>,
    pub(crate) request_data: Option<Params>,
    pub(crate) logging: Params,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct EditPlan {
    pub(crate) url: String,
    pub(crate) data: Params,
    pub(crate) files: Option<RequestFiles>,
    pub(crate) refusal: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExtensionPlan {
    pub(crate) url: String,
    pub(crate) data: Params,
    pub(crate) refusal: Option<String>,
}

// The provider config the handlers see: every method records what it was handed and answers from these fields
pub(crate) struct FakeConfig {
    pub(crate) events: Events,
    pub(crate) validation: Headers,
    pub(crate) base: Option<String>,
    pub(crate) prefetch: Option<(String, Params)>,
    pub(crate) edit: Option<EditPlan>,
    pub(crate) extension: Option<ExtensionPlan>,
    pub(crate) response_refusal: Option<String>,
    pub(crate) validated: Mutex<Option<Validated>>,
    pub(crate) url_seen: Mutex<Option<UrlSeen>>,
    pub(crate) prefetch_seen: Mutex<Option<PrefetchSeen>>,
    pub(crate) edit_seen: Mutex<Option<EditSeen>>,
    pub(crate) extension_seen: Mutex<Option<ExtensionSeen>>,
    pub(crate) edit_reply_seen: Mutex<Option<ReplySeen>>,
    pub(crate) extension_reply_seen: Mutex<Option<ReplySeen>>,
    pub(crate) errors: Mutex<Vec<(String, u16, Headers)>>,
}

impl FakeConfig {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
            validation: Headers::new(),
            base: None,
            prefetch: None,
            edit: None,
            extension: None,
            response_refusal: None,
            validated: Mutex::default(),
            url_seen: Mutex::default(),
            prefetch_seen: Mutex::default(),
            edit_seen: Mutex::default(),
            extension_seen: Mutex::default(),
            edit_reply_seen: Mutex::default(),
            extension_reply_seen: Mutex::default(),
            errors: Mutex::default(),
        }
    }

    pub(crate) fn validated(&self) -> Option<Validated> {
        self.validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn url_seen(&self) -> Option<UrlSeen> {
        self.url_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn prefetch_seen(&self) -> Option<PrefetchSeen> {
        self.prefetch_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn edit_seen(&self) -> Option<EditSeen> {
        self.edit_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn extension_seen(&self) -> Option<ExtensionSeen> {
        self.extension_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn edit_reply_seen(&self) -> Option<ReplySeen> {
        self.edit_reply_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn extension_reply_seen(&self) -> Option<ReplySeen> {
        self.extension_reply_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, Headers)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn refuse(&self, message: &str) -> BoxError {
        Box::new(Refusal {
            message: message.to_owned(),
        })
    }

    fn answer(&self) -> Result<VideoObject, BoxError> {
        if let Some(message) = &self.response_refusal {
            return Err(self.refuse(message));
        }
        Ok(VideoObject::new(
            "video-1".to_owned(),
            "completed".to_owned(),
        ))
    }
}

impl BaseVideoConfig for FakeConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        *self
            .validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Validated {
            headers: request.headers.clone(),
            model: request.model.to_owned(),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            params: request.litellm_params.cloned(),
        });
        self.validation.clone()
    }

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, BoxError> {
        *self.url_seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(UrlSeen {
            model: model.to_owned(),
            api_base: api_base.map(ToOwned::to_owned),
        });
        self.base.clone().ok_or_else(|| {
            Box::new(Refusal {
                message: "the fixture has no api base".to_owned(),
            })
        })
    }

    fn get_video_edit_prefetch_params(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Option<(String, Params)> {
        *self
            .prefetch_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(PrefetchSeen {
            video_id: video_id.to_owned(),
            api_base: context.api_base.to_owned(),
            headers: context.headers.clone(),
        });
        self.prefetch.clone()
    }

    fn transform_video_edit_request(
        &self,
        input: VideoEditInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params, Option<RequestFiles>), BoxError> {
        self.events.push("edit-transform");
        *self
            .edit_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(EditSeen {
            prompt: input.prompt.to_owned(),
            video_id: input.video_id.to_owned(),
            video_file: input.video_file.is_some(),
            extra_body: input.extra_body.cloned(),
            prefetched: input.prefetched_source_data.cloned(),
            api_base: context.api_base.to_owned(),
            headers: context.headers.clone(),
        });
        let Some(plan) = &self.edit else {
            return Err(self.refuse("the fixture has no edit plan"));
        };
        if let Some(message) = &plan.refusal {
            return Err(self.refuse(message));
        }
        Ok((plan.url.clone(), plan.data.clone(), plan.files.clone()))
    }

    fn transform_video_edit_response(
        &self,
        reply: VideoReply<'_>,
        request_data: Option<&Params>,
    ) -> Result<VideoObject, BoxError> {
        self.events.push("edit-response");
        *self
            .edit_reply_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(ReplySeen {
            status: reply.raw_response.status,
            body: reply.raw_response.body.clone(),
            provider: reply.custom_llm_provider.map(ToOwned::to_owned),
            request_data: request_data.cloned(),
            logging: reply.logging_obj.model_call_details.clone(),
        });
        self.answer()
    }

    fn transform_video_extension_request(
        &self,
        input: VideoExtensionInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), BoxError> {
        self.events.push("extension-transform");
        *self
            .extension_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(ExtensionSeen {
            prompt: input.prompt.to_owned(),
            video_id: input.video_id.to_owned(),
            seconds: input.seconds.to_owned(),
            extra_body: input.extra_body.cloned(),
            api_base: context.api_base.to_owned(),
            headers: context.headers.clone(),
        });
        let Some(plan) = &self.extension else {
            return Err(self.refuse("the fixture has no extension plan"));
        };
        if let Some(message) = &plan.refusal {
            return Err(self.refuse(message));
        }
        Ok((plan.url.clone(), plan.data.clone()))
    }

    fn transform_video_extension_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, BoxError> {
        self.events.push("extension-response");
        *self
            .extension_reply_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(ReplySeen {
            status: reply.raw_response.status,
            body: reply.raw_response.body.clone(),
            provider: reply.custom_llm_provider.map(ToOwned::to_owned),
            request_data: None,
            logging: reply.logging_obj.model_call_details.clone(),
        });
        self.answer()
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((error_message.to_owned(), status_code, headers.clone()));
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

// One call's inputs and its recording collaborators; a case changes only the fields it exercises
pub(crate) struct Harness<Client> {
    pub(crate) events: Events,
    pub(crate) source: FakeSource<Client>,
    pub(crate) log: FakeLog,
    pub(crate) config: FakeConfig,
    pub(crate) converter: SharedErrorConvert,
    pub(crate) params: Params,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) is_async: bool,
    pub(crate) supplied: Option<(ClientKind, Client)>,
    pub(crate) prompt: String,
    pub(crate) video_id: String,
    pub(crate) provider: String,
    pub(crate) seconds: String,
    pub(crate) video_file: Option<SuppliedMedia>,
}

impl<Client: WriteTransport + Clone> Harness<Client> {
    pub(crate) fn new(events: &Events, client: Client) -> Self {
        let mut config = FakeConfig::new(events);
        config.base = Some("https://upstream.test".to_owned());
        config.edit = Some(EditPlan {
            url: "https://upstream.test/edits".to_owned(),
            data: data(&[("prompt", Json::Text("make the sky brighter".to_owned()))]),
            files: None,
            refusal: None,
        });
        config.extension = Some(ExtensionPlan {
            url: "https://upstream.test/extensions".to_owned(),
            data: data(&[("seconds", Json::Text("4".to_owned()))]),
            refusal: None,
        });
        Self {
            events: events.clone(),
            source: FakeSource::new(client),
            log: FakeLog::new(events),
            config,
            converter: SharedErrorConvert,
            params: params(),
            extra_headers: None,
            extra_body: None,
            timeout: None,
            api_key: None,
            is_async: false,
            supplied: None,
            prompt: "make the sky brighter".to_owned(),
            video_id: "video-77".to_owned(),
            provider: KNOWN_PROVIDER.to_owned(),
            seconds: "4".to_owned(),
            video_file: None,
        }
    }

    pub(crate) const fn transport(
        &self,
    ) -> VideoTransport<'_, FakeSource<Client>, FakeLog, SharedErrorConvert> {
        VideoTransport::new(&self.source, &self.log, &self.converter)
    }

    pub(crate) fn edit_request(&self) -> VideoEditRequest<'_, Client> {
        VideoEditRequest {
            prompt: &self.prompt,
            video_id: &self.video_id,
            provider: &self.provider,
            litellm_params: &self.params,
            video_file: self.video_file.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_body: self.extra_body.as_ref(),
            timeout: self.timeout,
            client: self.supplied.as_ref().map(|(kind, client)| SuppliedClient {
                kind: *kind,
                transport: client,
            }),
            api_key: self.api_key.as_ref(),
            is_async: self.is_async,
        }
    }

    pub(crate) fn extension_request(&self) -> VideoExtensionRequest<'_, Client> {
        VideoExtensionRequest {
            prompt: &self.prompt,
            video_id: &self.video_id,
            seconds: &self.seconds,
            provider: &self.provider,
            litellm_params: &self.params,
            extra_headers: self.extra_headers.as_ref(),
            extra_body: self.extra_body.as_ref(),
            timeout: self.timeout,
            client: self.supplied.as_ref().map(|(kind, client)| SuppliedClient {
                kind: *kind,
                transport: client,
            }),
            api_key: self.api_key.as_ref(),
            is_async: self.is_async,
        }
    }
}

pub(crate) fn params() -> Params {
    let mut params = Params::new();
    params.insert(
        "api_base".to_owned(),
        Json::Text("https://upstream.test".to_owned()),
    );
    params
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn data(pairs: &[(&str, Json)]) -> Params {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn json_text(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture JSON parses")
}

pub(crate) fn object(text: &str) -> Params {
    let Json::Object(map) = json_text(text) else {
        panic!("the fixture is not a JSON object");
    };
    map
}

pub(crate) fn body_bytes(request: &WriteRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &WriteRequest) -> Json {
    let body = body_bytes(request).expect("the request carries a JSON body");
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) fn done(handled: Handled<'_>) -> VideoObject {
    match handled {
        Handled::Done(object) => *object,
        Handled::Pending(_) => panic!("the sync handler handed off a future"),
    }
}

pub(crate) fn provider_error(error: VideoEditTransportError) -> ProviderError {
    match error {
        VideoEditTransportError::Provider(inner) => *inner,
        other => panic!("expected a provider error, got {other:?}"),
    }
}

pub(crate) fn expect_created(handled: Result<Handled<'_>, VideoEditTransportError>) -> VideoObject {
    done(handled.unwrap_or_else(|error| panic!("the call failed: {error}")))
}

pub(crate) fn expect_failure(
    handled: Result<Handled<'_>, VideoEditTransportError>,
) -> VideoEditTransportError {
    handled
        .err()
        .unwrap_or_else(|| panic!("the call succeeded"))
}

pub(crate) fn run<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
        .block_on(future)
}

pub(crate) fn assert_nothing_sent(serving: Serving) {
    assert!(
        matches!(serving.finish(), Err(FixtureError::NoRequest)),
        "the server received a request"
    );
}

pub(crate) fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(
        base_builder().http1_only(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .expect("the proxied client builds")
}

pub(crate) fn serve_once(status: &str, headers: &[(&str, &str)], body: &str) -> Serving {
    serve(reply(status, headers, body), Duration::ZERO).expect("the fixture server serves")
}
