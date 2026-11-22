pub mod error;
pub mod handler;
pub mod providers;

mod logging;
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

use self::error::{VideoContentCallError, failure};
use self::handler::{ContentHandlerEnvironment, ContentHandlerRequest, VideoContentHandler};
use self::logging::log_call;
use self::providers::VideoContentProviders;
use crate::api::videos_http_transport::client::{ClientSource, SuppliedClient};
use crate::api::videos_http_transport::video_config::{Headers, Params};

// LiteLLM's fallback when neither the caller nor the video id names a provider
pub const DEFAULT_PROVIDER: &str = "openai";
// the kwargs key LiteLLM pops before anything else reads them
pub const ASYNC_CALL: &str = "async_call";

// video_content's declared arguments: the timeout's default is none, and extra_query and extra_body reach nothing
pub struct VideoContentRequest {
    pub video_id: String,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<String>,
    pub variant: Option<String>,
    pub extra_headers: Option<Headers>,
    // accepted to match LiteLLM's signature, which never forwards either to the content handler
    pub extra_query: Option<Params>,
    pub extra_body: Option<Params>,
    pub kwargs: IndexMap<String, Json>,
}

// what the sync entry hands back: the bytes, or the handler future async_call=true asked it to return
pub enum VideoContentWork<'call> {
    Ready(Vec<u8>),
    Pending(PendingContent<'call>),
}

pub type PendingContent<'call> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, VideoContentCallError>> + Send + 'call>>;

// the injected collaborators: B3332's selector, the B3333 handler, B185/B186's redactor and B198's request timeout
pub struct VideoContentCaller<Providers, Handler> {
    pub providers: Providers,
    pub handler: Handler,
    pub redactor: Redactor,
    pub request_timeout: Duration,
}

struct Prepared<'call, Providers: VideoContentProviders> {
    provider: String,
    is_async: bool,
    video_id: String,
    config: &'call Providers::Config,
    source: &'call Providers::Source,
    params: Params,
    extra_headers: Option<Headers>,
    variant: Option<String>,
    timeout: Duration,
}

impl<Providers, Handler> VideoContentCaller<Providers, Handler>
where
    Providers: VideoContentProviders,
    Handler: VideoContentHandler<Providers::Config, Providers::Source>,
{
    // LiteLLM's video_content: the async flag, the provider, the config, the logging update, then the handler
    pub fn video_content<'call>(
        &'call self,
        request: VideoContentRequest,
        client: Option<SuppliedClient<'call, <Providers::Source as ClientSource>::Client>>,
        logging: &'call mut LoggingState,
    ) -> Result<VideoContentWork<'call>, VideoContentCallError> {
        let prepared = self.prepare(request, logging)?;
        let provider = prepared.provider.clone();
        let environment = ContentHandlerEnvironment {
            config: prepared.config,
            source: prepared.source,
            logging,
            redactor: &self.redactor,
        };
        let handler_request = ContentHandlerRequest {
            video_id: prepared.video_id,
            custom_llm_provider: prepared.provider,
            litellm_params: prepared.params,
            extra_headers: prepared.extra_headers,
            variant: prepared.variant,
            timeout: prepared.timeout,
            client,
        };
        if prepared.is_async {
            let pending = self.handler.handle(environment, handler_request, true);
            return Ok(VideoContentWork::Pending(pending));
        }
        match poll_once(self.handler.handle(environment, handler_request, false)) {
            Polled::Ready(outcome) => outcome.map(VideoContentWork::Ready),
            Polled::StillWaiting => Err(failure(
                VideoContentCallError::Suspended,
                &provider,
                &self.redactor,
            )),
        }
    }

    // LiteLLM's avideo_content: its own provider decode, then the sync call's handler future awaited
    pub async fn avideo_content<'call>(
        &'call self,
        mut request: VideoContentRequest,
        client: Option<SuppliedClient<'call, <Providers::Source as ClientSource>::Client>>,
        logging: &'call mut LoggingState,
    ) -> Result<Vec<u8>, VideoContentCallError> {
        if request.custom_llm_provider.is_none() {
            let decoded = decoded_provider(&request.video_id);
            request.custom_llm_provider = Some(decoded);
        }
        request
            .kwargs
            .insert(ASYNC_CALL.to_owned(), Json::Bool(true));
        let provider = request.custom_llm_provider.clone().unwrap_or_default();
        let awaiting = match self.video_content(request, client, logging) {
            Ok(VideoContentWork::Ready(content)) => Ok(content),
            Ok(VideoContentWork::Pending(pending)) => pending.await,
            Err(error) => Err(error),
        };
        awaiting.map_err(|error| failure(error, &provider, &self.redactor))
    }

    // steps 2 to 6 of LiteLLM's try block, in order, before the handler is handed anything
    fn prepare<'call>(
        &'call self,
        request: VideoContentRequest,
        logging: &mut LoggingState,
    ) -> Result<Prepared<'call, Providers>, VideoContentCallError> {
        let mut kwargs = request.kwargs;
        let is_async = matches!(kwargs.shift_remove(ASYNC_CALL), Some(Json::Bool(true)));
        let provider = request
            .custom_llm_provider
            .unwrap_or_else(|| decoded_provider(&request.video_id));
        let (config, source) = self.select(&provider)?;
        log_call(logging, &kwargs, &request.video_id, &provider)
            .map_err(|error| failure(error, &provider, &self.redactor))?;
        Ok(Prepared {
            provider,
            is_async,
            video_id: request.video_id,
            config,
            source,
            params: kwargs,
            extra_headers: request.extra_headers,
            variant: request.variant,
            timeout: content_timeout(request.timeout, self.request_timeout),
        })
    }

    // litellm.LlmProviders(custom_llm_provider), then B3332's get_provider_video_config(model=None, provider)
    fn select(
        &self,
        provider: &str,
    ) -> Result<(&Providers::Config, &Providers::Source), VideoContentCallError> {
        let provider_id = ProviderId::from_str(provider).map_err(|source| {
            failure(
                VideoContentCallError::UnknownProvider {
                    provider: provider.to_owned(),
                    source,
                },
                provider,
                &self.redactor,
            )
        })?;
        let selected = self.providers.video_config(provider_id).map_err(|source| {
            failure(
                VideoContentCallError::Selection(source),
                provider,
                &self.redactor,
            )
        })?;
        let Some(found) = selected else {
            return Err(failure(
                VideoContentCallError::Unsupported {
                    provider: provider.to_owned(),
                },
                provider,
                &self.redactor,
            ));
        };
        Ok(found)
    }
}

// decode_video_id_with_provider(video_id)'s provider, or "openai" when the decode gives none or an empty one
fn decoded_provider(video_id: &str) -> String {
    decode_video_id_with_provider(video_id)
        .custom_llm_provider
        .filter(|provider| !provider.is_empty())
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
}

// `timeout or REQUEST_TIMEOUT`: the content signature declares no default, so 0 and none both take B198's value
const fn content_timeout(timeout: Option<Duration>, request_timeout: Duration) -> Duration {
    match timeout {
        Some(given) if !given.is_zero() => given,
        _ => request_timeout,
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
