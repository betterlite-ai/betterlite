mod prepare;
mod request;
mod tests;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_redact::secret::Redactor;

use self::prepare::prepare;
use self::request::{Polled, handler_request, poll_once};
use super::error::{VideoGenerationCallError, VideoGenerationFailure, failure};
use super::handler::{
    GenerationHandlerEnvironment, GenerationHandlerRequest, VideoGenerationHandling,
};
use super::providers::VideoGenerationProviders;
use super::{ASYNC_CALL, DEFAULT_MODEL, LOGGING_OBJECT, VideoGenerationRequest};
use crate::api::video_records::VideoObject;
use crate::api::video_remix_call::shared::{MockDecodeError, mock_answer};
use crate::api::videos_http_transport::client::ClientSource;
use crate::api::videos_http_transport::video_config::Params;

const MOCK_RESPONSE: &str = "mock_response";
const MOCKED_OBJECT: &str = "a video object";

// what the sync entry hands back: the object, or the handler future async_call=true asked it to return
pub enum VideoGenerationWork<'call> {
    Ready(Box<VideoObject>),
    Pending(PendingVideoGeneration<'call>),
}

pub type PendingVideoGeneration<'call> =
    Pin<Box<dyn Future<Output = Result<VideoObject, VideoGenerationCallError>> + Send + 'call>>;

// B247's logging state and the pre/post environment its records are written through
pub struct GenerationCallEnvironment<'call, 'env> {
    pub logging: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// the injected collaborators: B3332's selector, the B3333 handler, B185/B186's redactor and B198's switches
pub struct VideoGenerationCaller<Providers, Handler> {
    pub providers: Providers,
    pub handler: Handler,
    pub redactor: Redactor,
    // B198's request timeout, the fallback for `timeout or DEFAULT_REQUEST_TIMEOUT`
    pub request_timeout: Duration,
    // B198's drop-params switch, which the provider mapping reads
    pub drop_params: bool,
}

// LiteLLM's mock_response short circuit on its own, for a caller with no provider or handler to hand (the oracle helper)
pub fn mock_video_response(kwargs: &Params) -> Result<Option<VideoObject>, MockDecodeError> {
    match kwargs.get(MOCK_RESPONSE) {
        None | Some(Json::Null) => Ok(None),
        Some(mock) => mock_answer::<VideoObject>(Some(mock), MOCKED_OBJECT),
    }
}

impl<Providers, Handler> VideoGenerationCaller<Providers, Handler>
where
    Providers: VideoGenerationProviders,
    Handler: VideoGenerationHandling<Providers::Config, Providers::Source>,
{
    // LiteLLM's video_generation: the mock short circuit, the resolution, the options, the logging update and the handler
    pub fn video_generation<'call>(
        &'call self,
        mut request: VideoGenerationRequest,
        client: Option<&'call <Providers::Source as ClientSource>::Client>,
        resolution: &ResolutionContext<'_>,
        environment: GenerationCallEnvironment<'call, '_>,
    ) -> Result<VideoGenerationWork<'call>, VideoGenerationFailure> {
        let GenerationCallEnvironment { logging, env } = environment;
        request.kwargs.shift_remove(LOGGING_OBJECT);
        let is_async = request.kwargs.shift_remove(ASYNC_CALL) == Some(Json::Bool(true));
        // the mock answers before any provider lookup and before the logging update
        if let Some(object) = self.mock_object(&request)? {
            return Ok(VideoGenerationWork::Ready(Box::new(object)));
        }
        let litellm_params = request.kwargs.clone();
        let prepared = prepare(self, &mut request, logging, resolution)?;
        let handler_environment = GenerationHandlerEnvironment {
            config: prepared.config,
            source: prepared.source,
            logging,
            env,
        };
        let outgoing = handler_request(request, client, is_async, litellm_params, prepared);
        self.invoke_handler(outgoing, handler_environment, is_async)
    }

    fn invoke_handler<'call>(
        &'call self,
        request: GenerationHandlerRequest<'call, Providers::Source>,
        environment: GenerationHandlerEnvironment<'call, '_, Providers::Config, Providers::Source>,
        is_async: bool,
    ) -> Result<VideoGenerationWork<'call>, VideoGenerationFailure> {
        if is_async {
            let pending = self.handler.handle(environment, request, true);
            return Ok(VideoGenerationWork::Pending(pending));
        }
        let model = request.model.clone();
        let provider = request.custom_llm_provider.clone();
        let polled = poll_once(self.handler.handle(environment, request, false));
        self.sync_result(polled, &model, &provider)
    }

    // LiteLLM's avideo_generation: its own provider inference, the sync path with async_call true, then the future awaited
    pub async fn avideo_generation<'call>(
        &'call self,
        mut request: VideoGenerationRequest,
        client: Option<&'call <Providers::Source as ClientSource>::Client>,
        resolution: &ResolutionContext<'_>,
        environment: GenerationCallEnvironment<'call, '_>,
    ) -> Result<VideoObject, VideoGenerationFailure> {
        self.infer_provider(&mut request, resolution)?;
        request
            .kwargs
            .insert(ASYNC_CALL.to_owned(), Json::Bool(true));
        let async_model = request.model.clone();
        let async_provider = request.custom_llm_provider.clone();
        // a sync-path failure already went through exception_type; only the handler's own error reaches this except
        match self.video_generation(request, client, resolution, environment)? {
            VideoGenerationWork::Ready(object) => Ok(*object),
            VideoGenerationWork::Pending(pending) => pending.await.map_err(|error| {
                failure(
                    error,
                    async_model.as_deref(),
                    async_provider.as_deref(),
                    &self.redactor,
                )
            }),
        }
    }

    // avideo_generation infers the provider itself when the caller named none, with api_base always none
    fn infer_provider(
        &self,
        request: &mut VideoGenerationRequest,
        resolution: &ResolutionContext<'_>,
    ) -> Result<(), VideoGenerationFailure> {
        if request.custom_llm_provider.is_some() {
            return Ok(());
        }
        let model = request
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .unwrap_or(DEFAULT_MODEL);
        let inferred = resolve(
            &ResolutionRequest {
                model,
                custom_llm_provider: None,
                // video_generation's locals never hold api_base, so the inference always sees none
                api_base: None,
                api_key: None,
            },
            resolution,
        )
        .map_err(|error| {
            failure(
                VideoGenerationCallError::Resolution(error),
                request.model.as_deref(),
                None,
                &self.redactor,
            )
        })?;
        request.custom_llm_provider = Some(inferred.provider.slug().to_owned());
        Ok(())
    }

    // the mock is read off the keywords before the provider lookup, so no config or client takes part
    fn mock_object(
        &self,
        request: &VideoGenerationRequest,
    ) -> Result<Option<VideoObject>, VideoGenerationFailure> {
        mock_video_response(&request.kwargs).map_err(|error| {
            failure(
                VideoGenerationCallError::Mock(error),
                request
                    .model
                    .as_deref()
                    .filter(|model| !model.is_empty())
                    .or(Some(DEFAULT_MODEL)),
                request.custom_llm_provider.as_deref(),
                &self.redactor,
            )
        })
    }

    // the blocking path: a sync handler answers at once, and one that never does is a fault, not a wait
    fn sync_result<'call>(
        &'call self,
        polled: Polled<Result<VideoObject, VideoGenerationCallError>>,
        model: &str,
        provider: &str,
    ) -> Result<VideoGenerationWork<'call>, VideoGenerationFailure> {
        match polled {
            Polled::Ready(Ok(video)) => Ok(VideoGenerationWork::Ready(Box::new(video))),
            Polled::Ready(Err(error)) => {
                Err(failure(error, Some(model), Some(provider), &self.redactor))
            }
            Polled::StillWaiting => Err(failure(
                VideoGenerationCallError::Suspended,
                Some(model),
                Some(provider),
                &self.redactor,
            )),
        }
    }
}
