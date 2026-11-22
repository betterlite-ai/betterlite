use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;

use super::PendingVideoStatus;
use super::error::status_failure;
use crate::api::video_status_transport::clients::ClientFactories;
use crate::api::video_status_transport::config::StatusConfig;
use crate::api::video_status_transport::handler::VideoStatusHandler;
use crate::api::video_status_transport::records::{
    Headers, Params, StatusCall, SuppliedClient, VideoStatusHandled,
};

// what the B3337 handler call reads from the caller: the provider config, the logging state, the redactor
pub struct StatusHandlerEnvironment<'call, Factories: ClientFactories> {
    pub config: &'call StatusConfig<'call, Factories>,
    pub logging: &'call mut LoggingState,
    pub redactor: &'call Redactor,
}

// every argument LiteLLM's video_status_handler is given; extra_query and api_key have no place here
pub struct StatusHandlerRequest<'call, Synchronous, Asynchronous> {
    pub video_id: String,
    pub custom_llm_provider: String,
    pub litellm_params: Params,
    pub extra_headers: Option<Headers>,
    pub extra_body: Option<Params>,
    pub timeout: Option<Duration>,
    pub client: Option<SuppliedClient<'call, Synchronous, Asynchronous>>,
}

// the B3337 handler behind an injectable seam, so the call's steps can be checked without a provider
pub trait VideoStatusHandling<Factories: ClientFactories>: Send + Sync {
    fn handle<'call>(
        &'call self,
        environment: StatusHandlerEnvironment<'call, Factories>,
        request: StatusHandlerRequest<'call, Factories::Synchronous, Factories::Asynchronous>,
        is_async: bool,
    ) -> PendingVideoStatus<'call>;
}

// B3337's handler is the production implementation, so the call holds it directly
impl<Factories: ClientFactories> VideoStatusHandling<Factories>
    for VideoStatusHandler<'_, Factories>
{
    fn handle<'call>(
        &'call self,
        environment: StatusHandlerEnvironment<'call, Factories>,
        request: StatusHandlerRequest<'call, Factories::Synchronous, Factories::Asynchronous>,
        is_async: bool,
    ) -> PendingVideoStatus<'call> {
        let StatusHandlerEnvironment {
            config,
            logging,
            redactor,
        } = environment;
        let provider = request.custom_llm_provider.clone();
        Box::pin(async move {
            let call = StatusCall {
                video_id: &request.video_id,
                custom_llm_provider: &request.custom_llm_provider,
                litellm_params: &request.litellm_params,
                logging,
                extra_headers: request.extra_headers.as_ref(),
                extra_body: request.extra_body.as_ref(),
                timeout: request.timeout,
                api_key: None,
                client: request.client,
            };
            let outcome = match self.video_status_handler(config, call, is_async) {
                Ok(VideoStatusHandled::Ready(video)) => Ok(video),
                Ok(VideoStatusHandled::Pending(pending)) => pending.await,
                Err(error) => Err(error),
            };
            outcome.map_err(|error| status_failure(&error, &provider, redactor))
        })
    }
}
