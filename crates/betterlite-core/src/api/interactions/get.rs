pub mod call;
pub mod error;

mod tests;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::interactions::base::Logging;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::get::GetInteractionConfig;
use betterlite_http::interactions::get::GetInteractionRequest;
use betterlite_http::interactions::get::asynchronous::async_get_interaction;
use betterlite_http::interactions::get::synchronous::get_interaction;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::call::prepare;
use self::error::{InteractionGetError, InteractionGetFailureError, failure};
use crate::api::agents::item::error::ProviderFailure;

// the caller's side of LiteLLM's get and aget, which take the id, the header, deadline and provider overrides
pub struct InteractionGetRequest<'request> {
    pub interaction_id: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub kwargs: &'request IndexMap<String, Json>,
}

// the collaborators a retrieval writes into: the shared clients, B247's logging state, B3492's pre-call record and exception_type's redactor
pub struct InteractionGetDeps<'deps> {
    pub clients: &'deps mut HttpClients,
    pub logging: &'deps mut LoggingState,
    pub http: &'deps mut Logging,
    pub redactor: &'deps Redactor,
}

// LiteLLM's sync get and async aget run the same steps and differ only in which transport answers
macro_rules! retrieval_entries {
    (sync $method:ident, $send:path) => {
        pub fn $method<Config>(
            config: &Config,
            deps: InteractionGetDeps<'_>,
            request: &InteractionGetRequest<'_>,
        ) -> Result<Config::Output, InteractionGetFailureError<Config::Error>>
        where
            Config: GetInteractionConfig,
            Config::Error: ProviderFailure,
        {
            let InteractionGetDeps {
                clients,
                logging,
                http,
                redactor,
            } = deps;
            let prepared = prepare(request, logging, redactor)?;
            let sent = GetInteractionRequest {
                interaction_id: request.interaction_id,
                litellm_params: &prepared.params,
                provider: prepared.client,
                extra_headers: request
                    .extra_headers
                    .iter()
                    .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
                    .collect(),
                timeout: request.timeout,
            };
            $send(config, clients, http, None, &sent).map_err(|carried| {
                failure(
                    InteractionGetError::from(carried),
                    Some(prepared.provider.as_str()),
                    redactor,
                )
            })
        }
    };
    (async $method:ident, $send:path) => {
        pub async fn $method<Config>(
            config: &Config,
            deps: InteractionGetDeps<'_>,
            request: &InteractionGetRequest<'_>,
        ) -> Result<Config::Output, InteractionGetFailureError<Config::Error>>
        where
            Config: GetInteractionConfig,
            Config::Error: ProviderFailure,
        {
            let InteractionGetDeps {
                clients,
                logging,
                http,
                redactor,
            } = deps;
            let prepared = prepare(request, logging, redactor)?;
            let sent = GetInteractionRequest {
                interaction_id: request.interaction_id,
                litellm_params: &prepared.params,
                provider: prepared.client,
                extra_headers: request
                    .extra_headers
                    .iter()
                    .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
                    .collect(),
                timeout: request.timeout,
            };
            $send(config, clients, http, None, &sent)
                .await
                .map_err(|carried| {
                    failure(
                        InteractionGetError::from(carried),
                        Some(prepared.provider.as_str()),
                        redactor,
                    )
                })
        }
    };
}

retrieval_entries!(sync get, get_interaction);
retrieval_entries!(async aget, async_get_interaction);
