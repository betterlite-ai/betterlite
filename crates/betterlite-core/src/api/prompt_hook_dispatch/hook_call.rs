use std::error::Error;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::prompt_contracts::PromptIgnoreFlags;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::cache_accounting::census::count_request_breakpoints;
use betterlite_prompts::cache_defaults::attribution::record_gateway_injection;
use betterlite_prompts::orchestration::prompt_orchestration::{
    ChatCompletionPrompt, ChatCompletionPromptRequest, PromptOrchestrationError,
};

use super::json_bridge::{prompt_json, prompt_object, wire_object};
use super::selection::{PromptSelectionRequest, select_prompt_manager};
use super::{PromptHookDispatch, PromptManager};

pub struct PromptCallRequest<'request, BackendError>
where
    BackendError: Error + Send + Sync + 'static,
{
    pub model: &'request str,
    pub messages: &'request [Json],
    pub non_default_params: &'request JsonObject,
    pub prompt_variables: Option<&'request JsonObject>,
    pub prompt_id: Option<&'request str>,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub manager: Option<&'request Arc<dyn PromptManager<BackendError> + Send + Sync>>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
    pub request_kwargs: Option<&'request mut JsonObject>,
    pub injected_for_every_deployment: bool,
}

pub struct AsyncPromptCallRequest<'request, BackendError>
where
    BackendError: Error + Send + Sync + 'static,
{
    pub call: PromptCallRequest<'request, BackendError>,
    pub tools: Option<&'request [JsonObject]>,
}

// LiteLLM's get_chat_completion_prompt: the explicit manager wins over selection, and errors pass through.
pub async fn get_chat_completion_prompt<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: PromptCallRequest<'_, BackendError>,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<BackendError>>
where
    BackendError: Error + Send + Sync + 'static,
{
    let dynamic = Json::Object(dispatch.state.callback_dynamic_params.clone());
    let selected = match request.manager {
        Some(manager) => Some(Arc::clone(manager)),
        None => select_prompt_manager(
            dispatch,
            &PromptSelectionRequest {
                model: request.model,
                non_default_params: request.non_default_params,
                tools: None,
                prompt_id: request.prompt_id,
                prompt_spec: request.prompt_spec,
                dynamic_callback_params: Some(&dynamic),
            },
        ),
    };
    let Some(manager) = selected else {
        return Ok(finish_without_manager(dispatch, &request));
    };
    let before = breakpoints(request.messages);
    let prompt = manager
        .get_chat_completion_prompt(manager_request(&request, &dynamic))
        .await?;
    Ok(finish_hook_call(
        dispatch,
        request.request_kwargs,
        before,
        request.injected_for_every_deployment,
        prompt,
    ))
}

// LiteLLM's async_get_chat_completion_prompt; only the async call passes tools to the selection.
pub async fn async_completion_prompt<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: AsyncPromptCallRequest<'_, BackendError>,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<BackendError>>
where
    BackendError: Error + Send + Sync + 'static,
{
    let AsyncPromptCallRequest { call, tools } = request;
    let dynamic = Json::Object(dispatch.state.callback_dynamic_params.clone());
    let selected = match call.manager {
        Some(manager) => Some(Arc::clone(manager)),
        None => select_prompt_manager(
            dispatch,
            &PromptSelectionRequest {
                model: call.model,
                non_default_params: call.non_default_params,
                tools,
                prompt_id: call.prompt_id,
                prompt_spec: call.prompt_spec,
                dynamic_callback_params: Some(&dynamic),
            },
        ),
    };
    let Some(manager) = selected else {
        return Ok(finish_without_manager(dispatch, &call));
    };
    let before = breakpoints(call.messages);
    let prompt = manager
        .async_completion_prompt(manager_request(&call, &dynamic))
        .await?;
    Ok(finish_hook_call(
        dispatch,
        call.request_kwargs,
        before,
        call.injected_for_every_deployment,
        prompt,
    ))
}

fn finish_without_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptCallRequest<'_, BackendError>,
) -> ChatCompletionPrompt
where
    BackendError: Error + Send + Sync + 'static,
{
    let messages = request.messages.to_vec();
    dispatch.state.messages = Some(messages.clone());
    ChatCompletionPrompt {
        model: request.model.to_owned(),
        messages,
        non_default_params: request.non_default_params.clone(),
    }
}

fn finish_hook_call<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request_kwargs: Option<&mut JsonObject>,
    breakpoints_before: usize,
    injected_for_every_deployment: bool,
    prompt: ChatCompletionPrompt,
) -> ChatCompletionPrompt
where
    BackendError: Error + Send + Sync + 'static,
{
    dispatch.state.messages = Some(prompt.messages.clone());
    if let Some(kwargs) = request_kwargs {
        let after = breakpoints(&prompt.messages);
        let mut marked = prompt_object(kwargs);
        let added = i32::try_from(after.saturating_sub(breakpoints_before)).unwrap_or(i32::MAX);
        record_gateway_injection(&mut marked, added, injected_for_every_deployment);
        *kwargs = wire_object(&marked);
    }
    prompt
}

fn manager_request<'request, BackendError>(
    request: &'request PromptCallRequest<'request, BackendError>,
    dynamic_callback_params: &'request Json,
) -> ChatCompletionPromptRequest<'request>
where
    BackendError: Error + Send + Sync + 'static,
{
    ChatCompletionPromptRequest {
        model: request.model,
        messages: request.messages.to_vec(),
        non_default_params: request.non_default_params.clone(),
        prompt_id: request.prompt_id,
        prompt_variables: request.prompt_variables,
        dynamic_callback_params,
        prompt_spec: request.prompt_spec,
        prompt_label: request.prompt_label,
        prompt_version: request.prompt_version,
        ignore_flags: PromptIgnoreFlags::default(),
    }
}

fn breakpoints(messages: &[Json]) -> usize {
    let converted = messages.iter().map(prompt_json).collect::<Vec<_>>();
    count_request_breakpoints(&converted, None)
}

mod tests;
