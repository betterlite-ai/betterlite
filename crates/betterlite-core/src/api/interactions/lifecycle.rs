pub mod call;
pub mod error;
pub mod request;

mod tests;

use std::error::Error as StdError;
use std::fmt::Display;
use std::mem::take;

use betterlite_http::interactions::base::Logging;
use betterlite_http::interactions::base::RequestHeaders;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::lifecycle::{
    InteractionLifecycleConfig, InteractionsHttpHandler, LifecycleError as HttpError,
    LifecycleRequest as HttpRequest,
};
use betterlite_interactions::polling::host::SettlementHost;
use betterlite_interactions::settlement::settle_before_delete;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;

use self::call::{Operation, Prepared, prepare, resolve_provider};
use self::error::{InteractionLifecycleError, InteractionLifecycleFailure, failure};
use self::request::{BackgroundSettlement, InteractionLifecycleRequest};
use crate::api::agents::item::error::ProviderFailure;

// what one operation writes into: the handler's client cache and pre-call record, B247's logging state, the redactor
pub struct InteractionLifecycleDeps<'deps> {
    // owned because the handler holds it, and betterlite-core cannot name the reqwest client type its methods resolve
    pub clients: HttpClients,
    pub http: &'deps mut Logging,
    pub logging: &'deps mut LoggingState,
    pub redactor: &'deps Redactor,
}

// LiteLLM's delete, cancel and acancel prepare the request the same way and differ in the operation and the transport that sends it
macro_rules! lifecycle_entries {
    (sync $name:ident, $send:ident, $operation:ident, $output:ident) => {
        pub fn $name<Config>(
            config: &Config,
            deps: InteractionLifecycleDeps<'_>,
            request: &InteractionLifecycleRequest<'_>,
        ) -> Result<Config::$output, InteractionLifecycleFailure<Config::Error>>
        where
            Config: InteractionLifecycleConfig,
            Config::Error: ProviderFailure,
        {
            let InteractionLifecycleDeps {
                clients,
                http,
                logging,
                redactor,
            } = deps;
            let prepared = prepare(request, Operation::$operation, logging, redactor)?;
            let sent = http_request(request, &prepared);
            let mut handler = InteractionsHttpHandler {
                config,
                clients,
                logging: take(http),
                supplied: None,
            };
            let sent_result = handler.$send(&sent);
            *http = handler.logging;
            sent_result.map_err(|carried| finish(carried, &prepared, redactor))
        }
    };
    (async $name:ident, $send:ident, $operation:ident, $output:ident) => {
        pub async fn $name<Config>(
            config: &Config,
            deps: InteractionLifecycleDeps<'_>,
            request: &InteractionLifecycleRequest<'_>,
        ) -> Result<Config::$output, InteractionLifecycleFailure<Config::Error>>
        where
            Config: InteractionLifecycleConfig,
            Config::Error: ProviderFailure,
        {
            let InteractionLifecycleDeps {
                clients,
                http,
                logging,
                redactor,
            } = deps;
            let prepared = prepare(request, Operation::$operation, logging, redactor)?;
            let sent = http_request(request, &prepared);
            let mut handler = InteractionsHttpHandler {
                config,
                clients,
                logging: take(http),
                supplied: None,
            };
            let sent_result = handler.$send(&sent).await;
            *http = handler.logging;
            sent_result.map_err(|carried| finish(carried, &prepared, redactor))
        }
    };
}

lifecycle_entries!(sync delete, delete_interaction, Delete, DeleteOutput);

pub async fn adelete<Config, Host, FetchError>(
    config: &Config,
    deps: InteractionLifecycleDeps<'_>,
    request: &InteractionLifecycleRequest<'_>,
    settlement: BackgroundSettlement<'_, Host, FetchError>,
) -> Result<Config::DeleteOutput, InteractionLifecycleFailure<Config::Error, Host::BillingError>>
where
    Config: InteractionLifecycleConfig,
    Config::Error: ProviderFailure,
    Host: SettlementHost + Clone,
    FetchError: Display,
{
    let InteractionLifecycleDeps {
        clients,
        http,
        logging,
        redactor,
    } = deps;
    let provider = resolve_provider(request);
    settle_before_delete(
        settlement.registry,
        request.interaction_id,
        settlement.fetch,
    )
    .await
    .map_err(|error| {
        failure(
            InteractionLifecycleError::Settlement(error),
            &provider,
            redactor,
        )
    })?;
    let prepared = prepare(request, Operation::Delete, logging, redactor)?;
    let sent = http_request(request, &prepared);
    let mut handler = InteractionsHttpHandler {
        config,
        clients,
        logging: take(http),
        supplied: None,
    };
    let sent_result = handler.async_delete_interaction(&sent).await;
    *http = handler.logging;
    sent_result.map_err(|carried| finish(carried, &prepared, redactor))
}

lifecycle_entries!(sync cancel, cancel_interaction, Cancel, CancelOutput);
lifecycle_entries!(async acancel, async_cancel_interaction, Cancel, CancelOutput);

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
fn finish<Provider, Settlement>(
    carried: HttpError<Provider>,
    prepared: &Prepared,
    redactor: &Redactor,
) -> InteractionLifecycleFailure<Provider, Settlement>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    failure(
        InteractionLifecycleError::from(carried),
        &prepared.provider_name,
        redactor,
    )
}

fn http_request<'sent>(
    request: &'sent InteractionLifecycleRequest<'_>,
    prepared: &'sent Prepared,
) -> HttpRequest<'sent> {
    let extra_headers: RequestHeaders = request
        .extra_headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    HttpRequest {
        interaction_id: request.interaction_id,
        litellm_params: &prepared.params,
        provider: prepared.provider,
        extra_headers,
        timeout: request.timeout,
        request_timeout: prepared.deadline,
    }
}
