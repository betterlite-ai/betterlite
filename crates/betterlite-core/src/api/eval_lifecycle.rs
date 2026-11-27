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
use self::prepare::{EvalAction, Prepared, prepare};
use crate::api::eval_http::error::EvalHttpError;
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

// The collaborators an eval lifecycle call reads: the provider factory, the pooled client, B187's redactor and B198's request timeout
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

impl EvalLifecycleCall<'_> {
    // the same call over a copy of its keywords that carries the flag an async wrapper leaves for prepare to pop
    const fn flagged<'flagged>(
        &'flagged self,
        kwargs: &'flagged IndexMap<String, Json>,
    ) -> EvalLifecycleCall<'flagged> {
        EvalLifecycleCall {
            eval_id: self.eval_id,
            extra_headers: self.extra_headers,
            extra_query: self.extra_query,
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider,
            kwargs,
        }
    }
}

pub type EvalDeleteResult = Result<DeleteEvalResponse, EvalLifecycleFailure>;
pub type EvalCancelResult = Result<CancelEvalResponse, EvalLifecycleFailure>;

// delete/cancel and their async wrappers: the provider factory and the logging state are passed once
pub struct EvalLifecycles<'deps, 'state, 'env, Configs: ?Sized> {
    pub deps: &'deps EvalLifecycleDeps<'deps, Configs>,
    pub log: &'deps mut EvalLogging<'state, 'env>,
}

impl<Configs: EvalLifecycleConfigLookup + ?Sized> EvalLifecycles<'_, '_, '_, Configs> {
    // LiteLLM's delete_eval: prepare, connect, send, then map the handler's failure
    pub async fn delete_eval(&mut self, call: &EvalLifecycleCall<'_>) -> EvalDeleteResult {
        let prepared = prepare(self.deps, call, EvalAction::Delete, self.log.state)?;
        let connection = connect(self.deps, &prepared);
        let reply = delete_eval_handler(&connection, self.log, Some(prepared.timeout)).await;
        mapped(reply, &prepared, self.deps)
    }

    // adelete_eval sets the flag itself, as LiteLLM does before calling delete_eval
    pub async fn adelete_eval(&mut self, call: &EvalLifecycleCall<'_>) -> EvalDeleteResult {
        let mut kwargs = call.kwargs.clone();
        kwargs.insert(String::from(DELETE_ASYNC_FLAG), Json::Bool(true));
        self.delete_eval(&call.flagged(&kwargs)).await
    }

    // LiteLLM's cancel_eval: the same steps with the cancel transform and handler
    pub async fn cancel_eval(&mut self, call: &EvalLifecycleCall<'_>) -> EvalCancelResult {
        let prepared = prepare(self.deps, call, EvalAction::Cancel, self.log.state)?;
        let connection = connect(self.deps, &prepared);
        let reply = cancel_eval_handler(&connection, self.log, Some(prepared.timeout)).await;
        mapped(reply, &prepared, self.deps)
    }

    // acancel_eval sets the flag itself, as LiteLLM does before calling cancel_eval
    pub async fn acancel_eval(&mut self, call: &EvalLifecycleCall<'_>) -> EvalCancelResult {
        let mut kwargs = call.kwargs.clone();
        kwargs.insert(String::from(CANCEL_ASYNC_FLAG), Json::Bool(true));
        self.cancel_eval(&call.flagged(&kwargs)).await
    }
}

// the connection a handler call holds: this call's config, URL and headers, and the shared client
fn connect<'call, Configs>(
    deps: &'call EvalLifecycleDeps<'_, Configs>,
    prepared: &'call Prepared<Configs::Config>,
) -> EvalConnection<'call>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    EvalConnection {
        config: &prepared.config,
        client: deps.client,
        url: &prepared.url,
        headers: &prepared.headers,
    }
}

// the handler's reply, or the failure exception_type maps its error to, named and redacted here
fn mapped<Reply, Configs>(
    reply: Result<Reply, EvalHttpError>,
    prepared: &Prepared<Configs::Config>,
    deps: &EvalLifecycleDeps<'_, Configs>,
) -> Result<Reply, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    reply.map_err(|error| {
        failure(
            EvalLifecycleError::Handler(error),
            &prepared.provider,
            deps.redactor,
        )
    })
}
