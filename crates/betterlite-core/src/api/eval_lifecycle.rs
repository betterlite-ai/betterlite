pub mod error;
mod prepare;
mod tests;

use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::BaseEvalsAPIConfig;
use betterlite_openai::evals::types::{CancelEvalResponse, DeleteEvalResponse};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{EvalLifecycleError, EvalLifecycleFailure, failure};
use self::prepare::{EvalAction, prepare};
use crate::api::eval_http::exchange::{EvalConnection, EvalLogging};
use crate::api::eval_http::handlers::{cancel_eval_handler, delete_eval_handler};

pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com";
pub(crate) const DEFAULT_PROVIDER: &str = "openai";
pub(crate) const DELETE_ASYNC_FLAG: &str = "adelete_eval";
pub(crate) const CANCEL_ASYNC_FLAG: &str = "acancel_eval";
pub(crate) const LITELLM_CALL_ID: &str = "litellm_call_id";

// ProviderConfigManager.get_provider_evals_api_config: an executable config per call, or None for a name with no evals API
pub trait EvalLifecycleConfigLookup: Send + Sync {
    type Config: BaseEvalsAPIConfig<Error = EvalsError> + Send + Sync;

    fn eval_config(&self, provider: &str) -> Option<Self::Config>;
}

// The collaborators an eval lifecycle call reads: the provider factory, the pooled client, B185's redactor and B198's request timeout
pub struct EvalLifecycleDeps<'deps, Configs: ?Sized> {
    pub configs: &'deps Configs,
    pub client: &'deps ProxiedClient,
    pub redactor: &'deps Redactor,
    // already resolved by the caller, as LiteLLM reads its request_timeout global once
    pub request_timeout: Duration,
}

// LiteLLM's delete_eval and cancel_eval arguments; extra_query is accepted and never read
#[derive(Clone, Copy)]
pub struct EvalLifecycleCall<'call> {
    pub eval_id: &'call str,
    pub extra_headers: Option<&'call Headers>,
    // accepted to match LiteLLM's signature, which never forwards it to the handler
    pub extra_query: Option<&'call Object>,
    // None takes B198's request timeout, as `timeout or request_timeout` does
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'call str>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// delete_eval and its adelete_eval wrapper are one async operation here, because betterlite's evals transport is async only
pub async fn delete_eval<Configs>(
    deps: &EvalLifecycleDeps<'_, Configs>,
    call: &EvalLifecycleCall<'_>,
    log: &mut EvalLogging<'_, '_>,
) -> Result<DeleteEvalResponse, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    let prepared = prepare(deps, call, EvalAction::Delete, log.state)?;
    let connection = EvalConnection {
        config: &prepared.config,
        client: deps.client,
        url: &prepared.url,
        headers: &prepared.headers,
    };
    delete_eval_handler(&connection, log, Some(prepared.timeout))
        .await
        .map_err(|error| {
            failure(
                EvalLifecycleError::Handler(error),
                &prepared.provider,
                deps.redactor,
            )
        })
}

// LiteLLM's adelete_eval: the async keyword, then the same body whose result it awaits
pub async fn adelete_eval<Configs>(
    deps: &EvalLifecycleDeps<'_, Configs>,
    call: &EvalLifecycleCall<'_>,
    log: &mut EvalLogging<'_, '_>,
) -> Result<DeleteEvalResponse, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    let kwargs = flagged_kwargs(call, DELETE_ASYNC_FLAG);
    let flagged = EvalLifecycleCall {
        eval_id: call.eval_id,
        extra_headers: call.extra_headers,
        extra_query: call.extra_query,
        timeout: call.timeout,
        custom_llm_provider: call.custom_llm_provider,
        kwargs: &kwargs,
    };
    delete_eval(deps, &flagged, log).await
}

// cancel_eval and its acancel_eval wrapper are one async operation here, because betterlite's evals transport is async only
pub async fn cancel_eval<Configs>(
    deps: &EvalLifecycleDeps<'_, Configs>,
    call: &EvalLifecycleCall<'_>,
    log: &mut EvalLogging<'_, '_>,
) -> Result<CancelEvalResponse, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    let prepared = prepare(deps, call, EvalAction::Cancel, log.state)?;
    let connection = EvalConnection {
        config: &prepared.config,
        client: deps.client,
        url: &prepared.url,
        headers: &prepared.headers,
    };
    cancel_eval_handler(&connection, log, Some(prepared.timeout))
        .await
        .map_err(|error| {
            failure(
                EvalLifecycleError::Handler(error),
                &prepared.provider,
                deps.redactor,
            )
        })
}

// LiteLLM's acancel_eval: the async keyword, then the same body whose result it awaits
pub async fn acancel_eval<Configs>(
    deps: &EvalLifecycleDeps<'_, Configs>,
    call: &EvalLifecycleCall<'_>,
    log: &mut EvalLogging<'_, '_>,
) -> Result<CancelEvalResponse, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    let kwargs = flagged_kwargs(call, CANCEL_ASYNC_FLAG);
    let flagged = EvalLifecycleCall {
        eval_id: call.eval_id,
        extra_headers: call.extra_headers,
        extra_query: call.extra_query,
        timeout: call.timeout,
        custom_llm_provider: call.custom_llm_provider,
        kwargs: &kwargs,
    };
    cancel_eval(deps, &flagged, log).await
}

// the keyword an async wrapper leaves in the call keywords, which prepare pops before GenericLiteLLMParams reads them
fn flagged_kwargs(call: &EvalLifecycleCall<'_>, flag: &str) -> IndexMap<String, Json> {
    let mut kwargs = call.kwargs.clone();
    kwargs.insert(flag.to_owned(), Json::Bool(true));
    kwargs
}
