use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

use crate::api::messages_chat_prepare::inspect::has_compaction_block;
use crate::api::messages_chat_prepare::prediction::{
    EditPrediction, predict_context_edit_execution, runnable_spec,
};
use crate::api::messages_compaction::convert::{SystemShapeError, prompt_json, system_prompt};
use crate::api::messages_compaction::dispatch::DispatchRequest;
use crate::api::messages_context_clear::error::ContextManagementError;
use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::completion::SummaryCompletion;
use crate::api::summary_history::client_history::client_compaction_history;
use crate::api::summary_history::error::SummaryHistoryError;

// the dispatcher's failure split by what the wrapper does with it: the client's own error, or any other exception
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchFailure {
    Context(ContextManagementError),
    Other(String),
}

// B972's editor dispatcher behind the wrapper: a run either answers a result, answers nothing, or fails
#[async_trait]
pub trait PolyfillDispatch: Send + Sync {
    async fn dispatch(
        &self,
        request: DispatchRequest<'_>,
    ) -> Result<Option<PolyfillResult>, DispatchFailure>;
}

// the caller's request as the /v1/messages handler has it before any history or edit runs
#[derive(Clone, Copy)]
pub struct ContextRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
    pub system: Option<&'request Json>,
    pub context_management_spec: Option<&'request Json>,
    pub additional_drop_params: Option<&'request Json>,
    pub litellm_metadata: Option<&'request Object>,
    pub llm_router: Option<&'request dyn SummaryCompletion>,
    pub user_api_key_auth: Option<&'request UserAPIKeyAuth>,
}

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    History(#[from] SummaryHistoryError),
    #[error(transparent)]
    Shape(#[from] SystemShapeError),
    #[error(transparent)]
    Context(#[from] ContextManagementError),
}

// ast-grep-ignore: no-long-names
pub async fn prepare_context_managed_request<Dispatcher>(
    request: ContextRequest<'_>,
    dispatcher: &Dispatcher,
) -> Result<Option<PolyfillResult>, PrepareError>
where
    Dispatcher: PolyfillDispatch,
{
    let prediction = predict_context_edit_execution(
        request.context_management_spec,
        request.additional_drop_params,
    );
    // a compact edit slices around the client block itself, so pre-collapsing here would starve its count
    let history = if prediction.compact {
        None
    } else {
        client_history(request.messages, request.system)?
    };
    let result = run_polyfill_if_enabled(request, &history, prediction, dispatcher).await?;
    if result.is_some() {
        return Ok(result);
    }
    // the safety net after a compact-only failure: a compact prediction never computed history above
    if prediction.compact {
        return client_history(request.messages, request.system);
    }
    Ok(history)
}

// _run_polyfill_if_enabled: a client error wins; any other failure is logged, then swallowed for compact only
async fn run_polyfill_if_enabled<Dispatcher>(
    request: ContextRequest<'_>,
    history: &Option<PolyfillResult>,
    prediction: EditPrediction,
    dispatcher: &Dispatcher,
) -> Result<Option<PolyfillResult>, PrepareError>
where
    Dispatcher: PolyfillDispatch,
{
    let Some(spec) = runnable_spec(
        request.context_management_spec,
        request.additional_drop_params,
    ) else {
        return Ok(None);
    };
    let (messages, system) = match history {
        Some(result) => (result.messages.clone(), result.system.clone()),
        None => (request.messages.to_vec(), request.system.cloned()),
    };
    match dispatcher
        .dispatch(dispatch_request(request, messages, system, spec))
        .await
    {
        Ok(result) => Ok(result),
        Err(DispatchFailure::Context(error)) => Err(PrepareError::Context(error)),
        Err(DispatchFailure::Other(message)) => recovered(&message, prediction),
    }
}

// the best-effort branch: the failure is always logged, and only a compact-only spec may hide it
fn recovered(
    message: &str,
    prediction: EditPrediction,
) -> Result<Option<PolyfillResult>, PrepareError> {
    tracing::error!("context_management polyfill: skipping edits due to error: {message}");
    if prediction.non_compact {
        return Err(PrepareError::Context(ContextManagementError {
            status_code: 500,
            message: format!("context_management polyfill failed: {message}"),
        }));
    }
    Ok(None)
}

// apply_client_compaction_block_history, gated so a system shape only matters when there is a block to honor
fn client_history(
    messages: &[Json],
    system: Option<&Json>,
) -> Result<Option<PolyfillResult>, PrepareError> {
    if !has_compaction_block(messages) {
        return Ok(None);
    }
    let converted: Vec<PromptJson> = messages.iter().map(prompt_json).collect();
    let prompt_system = system_prompt(system)?;
    let history = client_compaction_history(&converted, prompt_system.as_ref())?;
    Ok(history)
}

// the dispatcher's keyword arguments, with the history the wrapper chose and the caller's request-wide values
fn dispatch_request<'request>(
    request: ContextRequest<'request>,
    messages: Vec<Json>,
    system: Option<Json>,
    spec: &'request Json,
) -> DispatchRequest<'request> {
    DispatchRequest {
        model: request.model,
        messages,
        tools: request.tools.map(<[Json]>::to_vec),
        system,
        context_management_spec: Some(spec),
        litellm_metadata: request.litellm_metadata,
        llm_router: request.llm_router,
        user_api_key_auth: request.user_api_key_auth,
    }
}
