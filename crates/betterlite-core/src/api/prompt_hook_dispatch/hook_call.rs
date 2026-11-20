use std::error::Error;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::prompt_contracts::PromptIgnoreFlags;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::cache_accounting::census::count_request_breakpoints;
use betterlite_prompts::cache_defaults::attribution::record_gateway_injection;
use betterlite_prompts::orchestration::prompt_orchestration::{
    ChatCompletionPrompt, ChatCompletionPromptRequest, CompletionFuture, PromptOrchestrationError,
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
    pub manager: Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>,
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

// LiteLLM keeps the two hooks in separate methods; the mode picks which one a call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookMode {
    Sync,
    Async,
}

// LiteLLM's get_chat_completion_prompt: no tools reach the selection and the selected manager's sync hook runs.
pub async fn get_chat_completion_prompt<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: PromptCallRequest<'_, BackendError>,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<BackendError>> {
    let dynamic = Json::Object(dispatch.state.callback_dynamic_params.clone());
    let manager = explicit_or_selected(dispatch, &request, &dynamic, None);
    run_hook(dispatch, request, &dynamic, manager, HookMode::Sync).await
}

// LiteLLM's async_get_chat_completion_prompt: the tools go to the selection and the selected manager's async hook runs.
pub fn async_completion_prompt<'call, BackendError: Error + Send + Sync + 'static>(
    dispatch: &'call mut PromptHookDispatch<'_, BackendError>,
    request: AsyncPromptCallRequest<'call, BackendError>,
) -> CompletionFuture<'call, BackendError> {
    Box::pin(async move {
        let AsyncPromptCallRequest { call, tools } = request;
        let dynamic = Json::Object(dispatch.state.callback_dynamic_params.clone());
        let manager = explicit_or_selected(dispatch, &call, &dynamic, tools);
        run_hook(dispatch, call, &dynamic, manager, HookMode::Async).await
    })
}

async fn run_hook<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: PromptCallRequest<'_, BackendError>,
    dynamic_callback_params: &Json,
    manager: Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>,
    mode: HookMode,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<BackendError>> {
    let Some(manager) = manager else {
        return Ok(store_input(dispatch, &request));
    };
    let before = count_request_breakpoints(
        &request.messages.iter().map(prompt_json).collect::<Vec<_>>(),
        None,
    );
    let call = manager_request(&request, dynamic_callback_params);
    let prompt = match mode {
        HookMode::Sync => manager.get_chat_completion_prompt(call).await?,
        HookMode::Async => manager.async_completion_prompt(call).await?,
    };
    Ok(complete_call(
        dispatch,
        request.request_kwargs,
        before,
        request.injected_for_every_deployment,
        prompt,
    ))
}

fn explicit_or_selected<'call, BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptCallRequest<'call, BackendError>,
    dynamic_callback_params: &Json,
    tools: Option<&'call [JsonObject]>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>> {
    if let Some(manager) = &request.manager {
        return Some(Arc::clone(manager));
    }
    select_prompt_manager(
        dispatch,
        &PromptSelectionRequest {
            model: request.model,
            non_default_params: request.non_default_params,
            tools,
            prompt_id: request.prompt_id,
            prompt_spec: request.prompt_spec,
            dynamic_callback_params: Some(dynamic_callback_params),
        },
    )
}

fn store_input<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptCallRequest<'_, BackendError>,
) -> ChatCompletionPrompt {
    let messages = request.messages.to_vec();
    dispatch.state.messages = Some(messages.clone());
    ChatCompletionPrompt {
        model: request.model.to_owned(),
        messages,
        non_default_params: request.non_default_params.clone(),
    }
}

fn complete_call<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request_kwargs: Option<&mut JsonObject>,
    breakpoints_before: usize,
    injected_for_every_deployment: bool,
    prompt: ChatCompletionPrompt,
) -> ChatCompletionPrompt {
    dispatch.state.messages = Some(prompt.messages.clone());
    if let Some(kwargs) = request_kwargs {
        let after = count_request_breakpoints(
            &prompt.messages.iter().map(prompt_json).collect::<Vec<_>>(),
            None,
        );
        let added = i32::try_from(after.saturating_sub(breakpoints_before)).unwrap_or(i32::MAX);
        let mut marked = prompt_object(kwargs);
        record_gateway_injection(&mut marked, added, injected_for_every_deployment);
        *kwargs = wire_object(&marked);
    }
    prompt
}

fn manager_request<'request, BackendError: Error + Send + Sync + 'static>(
    request: &'request PromptCallRequest<'request, BackendError>,
    dynamic_callback_params: &'request Json,
) -> ChatCompletionPromptRequest<'request> {
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

mod tests;
