use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;

use super::call::PendingVideoGeneration;
use super::error::handler_error;
use super::providers::VideoGenerationConfig;
use crate::api::videos_http_transport::client::{ClientSource, SuppliedClient, VideoDeps};
use crate::api::videos_http_transport::generation::{
    LogContext, VideoGenerationCall, video_generation_handler,
};
use crate::api::videos_http_transport::video_config::{Headers, Params};

// what the B3333 handler call reads from the caller: the provider config, its client source, B247's state and the env
pub struct GenerationHandlerEnvironment<'call, 'env, Config, Source> {
    pub config: &'call Config,
    pub source: &'call Source,
    pub logging: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// every argument LiteLLM's video_generation_handler is given, owned so the pending future outlives the caller
pub struct GenerationHandlerRequest<'call, Source: ClientSource> {
    pub model: String,
    pub prompt: String,
    pub custom_llm_provider: String,
    pub litellm_params: Params,
    pub optional_params: Params,
    pub extra_headers: Option<Headers>,
    // the original argument, which LiteLLM carries into the handler and never reads on the sync path
    pub extra_body: Option<Params>,
    pub timeout: Duration,
    pub client: Option<SuppliedClient<'call, Source::Client>>,
}

// B3333's handler behind an injectable seam, so the call's steps can be checked without a provider
pub trait VideoGenerationHandling<Config, Source>: Send + Sync
where
    Config: VideoGenerationConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        environment: GenerationHandlerEnvironment<'call, '_, Config, Source>,
        request: GenerationHandlerRequest<'call, Source>,
        is_async: bool,
    ) -> PendingVideoGeneration<'call>;
}

// the production seam: B3333's handler is the call, and its failure travels back unmapped for the entry to classify
pub struct TransportGenerationHandler;

impl<Config, Source> VideoGenerationHandling<Config, Source> for TransportGenerationHandler
where
    Config: VideoGenerationConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        environment: GenerationHandlerEnvironment<'call, '_, Config, Source>,
        request: GenerationHandlerRequest<'call, Source>,
        is_async: bool,
    ) -> PendingVideoGeneration<'call> {
        let GenerationHandlerEnvironment {
            config,
            source,
            logging,
            env,
        } = environment;
        Box::pin(async move {
            let deps = VideoDeps {
                config,
                source,
                supplied: request.client,
            };
            let call = VideoGenerationCall {
                model: &request.model,
                prompt: &request.prompt,
                optional_params: &request.optional_params,
                custom_llm_provider: &request.custom_llm_provider,
                litellm_params: &request.litellm_params,
                extra_headers: request.extra_headers.as_ref(),
                timeout: request.timeout,
                api_key: None,
            };
            let mut log = LogContext {
                state: logging,
                env,
            };
            video_generation_handler(&deps, &call, &mut log, is_async)
                .await
                .map_err(|error| handler_error(&error, config))
        })
    }
}
