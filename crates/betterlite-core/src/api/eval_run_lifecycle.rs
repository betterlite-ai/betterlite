pub mod error;
pub mod handlers;
mod prepare;
mod tests;

use std::future::Future;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{CancelRunResponse, Run, RunDeleteResponse};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Handle};

use self::error::{EvalRunError, EvalRunFailure, RunAction, failure};
use self::handlers::EvalRunHandlers;
use self::prepare::{prepare, prepare_async};
use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::EvalConfig;

// LiteLLM's evals default provider when the caller names none
pub const DEFAULT_PROVIDER: &str = "openai";
// litellm/evals/main.py's DEFAULT_OPENAI_API_BASE, read when litellm_params.api_base is absent
pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com";

// The arguments get_run, cancel_run and delete_run take, with the loose keywords GenericLiteLLMParams reads
#[derive(Debug, Clone, Copy)]
pub struct EvalRunRequest<'call> {
    pub eval_id: &'call str,
    pub run_id: &'call str,
    pub extra_headers: Option<&'call Headers>,
    // accepted and never forwarded: it reaches local_vars in LiteLLM and no transform or provider call sees it
    pub extra_query: Option<&'call Object>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'call str>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// One call's dependencies: B9371's handlers, the evals config the provider factory would return, and the unset-call timeout
pub struct EvalRunDeps<'deps, Handler: EvalRunHandlers + ?Sized> {
    pub handler: &'deps Handler,
    pub config: EvalConfig<'deps>,
    pub redactor: &'deps Redactor,
    pub request_timeout: Duration,
}

// LiteLLM's get_run: the shared preparation, then the run handler driven on a runtime this call owns
pub fn get_run<Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'_, Handler>,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<Run, EvalRunFailure> {
    let prepared = prepare(deps, RunAction::Get, request, logging)?;
    let handler_request = prepared.handler_request();
    block(deps.handler.get_run(&handler_request, logging))
        .map_err(|error| failure(error, &prepared.provider, deps.redactor))
}

// LiteLLM's aget_run: the wrapper's kwargs flag, the same preparation, and the handler awaited on the caller's task
pub async fn aget_run<Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'_, Handler>,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<Run, EvalRunFailure> {
    let prepared = prepare_async(deps, RunAction::Get, request, logging)?;
    let handler_request = prepared.handler_request();
    deps.handler
        .get_run(&handler_request, logging)
        .await
        .map_err(|error| {
            failure(
                EvalRunError::Handler(error),
                &prepared.provider,
                deps.redactor,
            )
        })
}

// LiteLLM's cancel_run: the same body as get_run with the cancel transform and handler
pub fn cancel_run<Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'_, Handler>,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<CancelRunResponse, EvalRunFailure> {
    let prepared = prepare(deps, RunAction::Cancel, request, logging)?;
    let handler_request = prepared.handler_request();
    block(deps.handler.cancel_run(&handler_request, logging))
        .map_err(|error| failure(error, &prepared.provider, deps.redactor))
}

// LiteLLM's acancel_run: the cancel body with the wrapper's kwargs flag and the handler awaited
pub async fn acancel_run<Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'_, Handler>,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<CancelRunResponse, EvalRunFailure> {
    let prepared = prepare_async(deps, RunAction::Cancel, request, logging)?;
    let handler_request = prepared.handler_request();
    deps.handler
        .cancel_run(&handler_request, logging)
        .await
        .map_err(|error| {
            failure(
                EvalRunError::Handler(error),
                &prepared.provider,
                deps.redactor,
            )
        })
}

// LiteLLM's delete_run: the same body as get_run with the delete transform and handler
pub fn delete_run<Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'_, Handler>,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<RunDeleteResponse, EvalRunFailure> {
    let prepared = prepare(deps, RunAction::Delete, request, logging)?;
    let handler_request = prepared.handler_request();
    block(deps.handler.delete_run(&handler_request, logging))
        .map_err(|error| failure(error, &prepared.provider, deps.redactor))
}

// LiteLLM's adelete_run: the delete body with the wrapper's kwargs flag and the handler awaited
pub async fn adelete_run<Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'_, Handler>,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<RunDeleteResponse, EvalRunFailure> {
    let prepared = prepare_async(deps, RunAction::Delete, request, logging)?;
    let handler_request = prepared.handler_request();
    deps.handler
        .delete_run(&handler_request, logging)
        .await
        .map_err(|error| {
            failure(
                EvalRunError::Handler(error),
                &prepared.provider,
                deps.redactor,
            )
        })
}

// B9371's handlers are async only, so a blocking function drives one on a runtime it builds and owns
fn block<Answer>(
    future: impl Future<Output = Result<Answer, EvalHttpError>>,
) -> Result<Answer, EvalRunError> {
    if Handle::try_current().is_ok() {
        return Err(EvalRunError::SyncInRuntime);
    }
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| EvalRunError::Runtime { source })?;
    runtime.block_on(future).map_err(EvalRunError::Handler)
}
