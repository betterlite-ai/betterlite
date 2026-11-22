pub mod error;
pub mod handler;
pub mod providers;

mod logging;
mod mock;
mod tests;

use std::future::Future;
use std::pin::{Pin, pin};
use std::str::FromStr as _;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{VideoStatusCallError, failure};
use self::handler::{StatusHandlerEnvironment, StatusHandlerRequest, VideoStatusHandling};
use self::logging::log_call;
use self::mock::mock_response;
use self::providers::VideoStatusProviders;
use crate::api::video_records::VideoObject;
use crate::api::video_status_transport::clients::ClientFactories;
use crate::api::video_status_transport::config::StatusConfig;
use crate::api::video_status_transport::records::{Headers, Params, SuppliedClient};

// LiteLLM's fallback when neither the caller nor the video id names a provider
pub const DEFAULT_PROVIDER: &str = "openai";
// the kwargs key LiteLLM pops before anything else reads them
pub const ASYNC_CALL: &str = "async_call";
// video_status's declared timeout default, which the request's none stands for
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
// CallTypes.video_retrieve.value, which the async call records too
pub const CALL_TYPE: &str = "video_retrieve";

// video_status's declared arguments: the timeout's default is 600, and extra_query reaches nothing
pub struct VideoStatusRequest {
    pub video_id: String,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<Headers>,
    // accepted to match LiteLLM's signature, which never forwards extra_query to the status handler
    pub extra_query: Option<Params>,
    pub extra_body: Option<Params>,
    pub kwargs: IndexMap<String, Json>,
}

// what the sync entry hands back: the object, or the handler future async_call=true asked it to return
pub enum VideoStatusWork<'call> {
    Ready(VideoObject),
    Pending(PendingVideoStatus<'call>),
}

pub type PendingVideoStatus<'call> =
    Pin<Box<dyn Future<Output = Result<VideoObject, VideoStatusCallError>> + Send + 'call>>;

// the injected collaborators: B3332's selector, the B3337 handler, B185/B186's redactor and B198's request timeout
pub struct VideoStatusCaller<Providers, Handler> {
    pub providers: Providers,
    pub handler: Handler,
    pub redactor: Redactor,
    pub request_timeout: Duration,
}

struct Prepared<'call, Providers: VideoStatusProviders> {
    provider: String,
    video_id: String,
    config: &'call StatusConfig<'call, Providers::Factories>,
    params: Params,
    extra_headers: Option<Headers>,
    extra_body: Option<Params>,
    timeout: Duration,
}

impl<Providers, Handler> VideoStatusCaller<Providers, Handler>
where
    Providers: VideoStatusProviders,
    Handler: VideoStatusHandling<Providers::Factories>,
{
    // LiteLLM's video_status: the async flag, the mock response, the provider, the config, the logging update and call type, then the handler
    pub fn video_status<'call>(
        &'call self,
        mut request: VideoStatusRequest,
        client: Option<
            SuppliedClient<
                'call,
                <Providers::Factories as ClientFactories>::Synchronous,
                <Providers::Factories as ClientFactories>::Asynchronous,
            >,
        >,
        logging: &'call mut LoggingState,
    ) -> Result<VideoStatusWork<'call>, VideoStatusCallError> {
        let is_async = matches!(
            request.kwargs.shift_remove(ASYNC_CALL),
            Some(Json::Bool(true))
        );
        let named = request.custom_llm_provider.as_deref().unwrap_or_default();
        let mocked = mock_response(&request.kwargs)
            .map_err(|error| failure(error, named, &self.redactor))?;
        if let Some(video) = mocked {
            return Ok(VideoStatusWork::Ready(video));
        }
        let prepared = self.prepare(request, logging)?;
        let provider = prepared.provider.clone();
        let environment = StatusHandlerEnvironment {
            config: prepared.config,
            logging,
            redactor: &self.redactor,
        };
        let handler_request = StatusHandlerRequest {
            video_id: prepared.video_id,
            custom_llm_provider: prepared.provider,
            litellm_params: prepared.params,
            extra_headers: prepared.extra_headers,
            extra_body: prepared.extra_body,
            timeout: Some(prepared.timeout),
            client,
        };
        if is_async {
            let pending = self.handler.handle(environment, handler_request, true);
            return Ok(VideoStatusWork::Pending(pending));
        }
        match poll_once(self.handler.handle(environment, handler_request, false)) {
            Polled::Ready(outcome) => outcome.map(VideoStatusWork::Ready),
            Polled::StillWaiting => Err(failure(
                VideoStatusCallError::Suspended,
                &provider,
                &self.redactor,
            )),
        }
    }

    // LiteLLM's avideo_status: no decode of its own, and the sync call's handler future awaited
    pub async fn avideo_status<'call>(
        &'call self,
        mut request: VideoStatusRequest,
        client: Option<
            SuppliedClient<
                'call,
                <Providers::Factories as ClientFactories>::Synchronous,
                <Providers::Factories as ClientFactories>::Asynchronous,
            >,
        >,
        logging: &'call mut LoggingState,
    ) -> Result<VideoObject, VideoStatusCallError> {
        request
            .kwargs
            .insert(ASYNC_CALL.to_owned(), Json::Bool(true));
        let provider = request.custom_llm_provider.clone().unwrap_or_default();
        let awaiting = match self.video_status(request, client, logging) {
            Ok(VideoStatusWork::Ready(video)) => Ok(video),
            Ok(VideoStatusWork::Pending(pending)) => pending.await,
            Err(error) => Err(error),
        };
        awaiting.map_err(|error| failure(error, &provider, &self.redactor))
    }

    // steps 2 to 6 of LiteLLM's try block, in order, with the call type set before the handler is handed anything
    fn prepare<'call>(
        &'call self,
        request: VideoStatusRequest,
        logging: &mut LoggingState,
    ) -> Result<Prepared<'call, Providers>, VideoStatusCallError> {
        let provider = request
            .custom_llm_provider
            .unwrap_or_else(|| decoded_provider(&request.video_id));
        let config = self.select(&provider)?;
        log_call(logging, &request.kwargs, &request.video_id, &provider)
            .map_err(|error| failure(error, &provider, &self.redactor))?;
        CALL_TYPE.clone_into(&mut logging.call_type);
        Ok(Prepared {
            provider,
            video_id: request.video_id,
            config,
            params: request.kwargs,
            extra_headers: request.extra_headers,
            extra_body: request.extra_body,
            timeout: status_timeout(request.timeout, self.request_timeout),
        })
    }

    // litellm.LlmProviders(custom_llm_provider), then B3332's get_provider_video_config(model=None, provider)
    fn select<'call>(
        &'call self,
        provider: &str,
    ) -> Result<&'call StatusConfig<'call, Providers::Factories>, VideoStatusCallError> {
        let provider_id = ProviderId::from_str(provider).map_err(|source| {
            failure(
                VideoStatusCallError::UnknownProvider {
                    provider: provider.to_owned(),
                    source,
                },
                provider,
                &self.redactor,
            )
        })?;
        let selected = self.providers.video_config(provider_id).map_err(|source| {
            failure(
                VideoStatusCallError::Selection(source),
                provider,
                &self.redactor,
            )
        })?;
        let Some(config) = selected else {
            return Err(failure(
                VideoStatusCallError::Unsupported {
                    provider: provider.to_owned(),
                },
                provider,
                &self.redactor,
            ));
        };
        Ok(config)
    }
}

// decode_video_id_with_provider(video_id)'s provider, or "openai" when the decode gives none or an empty one
fn decoded_provider(video_id: &str) -> String {
    decode_video_id_with_provider(video_id)
        .custom_llm_provider
        .filter(|provider| !provider.is_empty())
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
}

// `timeout or REQUEST_TIMEOUT`: the declared default is 600, an explicit zero is falsey, and none is the default
const fn status_timeout(timeout: Option<Duration>, request_timeout: Duration) -> Duration {
    match timeout {
        Some(given) if !given.is_zero() => given,
        Some(_) => request_timeout,
        None => DEFAULT_TIMEOUT,
    }
}

enum Polled<Output> {
    Ready(Output),
    StillWaiting,
}

// the blocking path's single poll: a sync handler answers at once, so only a broken one leaves this waiting
fn poll_once<Running>(future: Running) -> Polled<Running::Output>
where
    Running: Future,
{
    let mut pinned = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match pinned.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => Polled::Ready(outcome),
        Poll::Pending => Polled::StillWaiting,
    }
}
