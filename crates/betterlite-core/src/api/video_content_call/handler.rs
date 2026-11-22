use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;

use super::PendingContent;
use super::error::transport_failure;
use crate::api::videos_http_transport::client::{ClientSource, SuppliedClient, VideoDeps};
use crate::api::videos_http_transport::content::{VideoContentCall, video_content_handler};
use crate::api::videos_http_transport::video_config::{Headers, Params, VideoHttpConfig};

// what the B3333 handler call reads from the caller: the provider config and source, the logging state, the redactor
pub struct ContentHandlerEnvironment<'call, Config, Source: ClientSource> {
    pub config: &'call Config,
    pub source: &'call Source,
    pub logging: &'call mut LoggingState,
    pub redactor: &'call Redactor,
}

// every argument LiteLLM's video_content_handler is given; extra_query, extra_body and api_key have no place here
pub struct ContentHandlerRequest<'call, Client> {
    pub video_id: String,
    pub custom_llm_provider: String,
    pub litellm_params: Params,
    pub extra_headers: Option<Headers>,
    pub variant: Option<String>,
    pub timeout: Duration,
    pub client: Option<SuppliedClient<'call, Client>>,
}

// the B3333 handler behind an injectable seam, so the call's steps can be checked without a wire
pub trait VideoContentHandler<Config, Source>: Send + Sync
where
    Config: VideoHttpConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        environment: ContentHandlerEnvironment<'call, Config, Source>,
        request: ContentHandlerRequest<'call, Source::Client>,
        is_async: bool,
    ) -> PendingContent<'call>;
}

// the library's one production handler: B3333's video_content_handler with its failure mapped by exception_type
pub struct ContentTransportHandler;

impl<Config, Source> VideoContentHandler<Config, Source> for ContentTransportHandler
where
    Config: VideoHttpConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        environment: ContentHandlerEnvironment<'call, Config, Source>,
        request: ContentHandlerRequest<'call, Source::Client>,
        is_async: bool,
    ) -> PendingContent<'call> {
        let ContentHandlerEnvironment {
            config,
            source,
            logging,
            redactor,
        } = environment;
        Box::pin(async move {
            let deps = VideoDeps {
                config,
                source,
                supplied: request.client,
            };
            let call = VideoContentCall {
                video_id: &request.video_id,
                custom_llm_provider: &request.custom_llm_provider,
                litellm_params: &request.litellm_params,
                extra_headers: request.extra_headers.as_ref(),
                api_key: None,
                variant: request.variant.as_deref(),
                timeout: request.timeout,
            };
            video_content_handler(&deps, &call, logging, is_async)
                .await
                .map_err(|error| {
                    transport_failure(error, &request.custom_llm_provider, config, redactor)
                })
        })
    }
}
