use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptCompileRequest, PromptIgnoreFlags, PromptManagementClient,
    PromptParams, PromptProcessingOptions, StandardCallbackDynamicParams,
    post_compile_prompt_processing, resolve_ignore_flags,
};
use betterlite_models::prompt_spec::PromptSpec;

use super::compilation::{compile_prompt_helper, should_run_prompt_management};
use super::{INTEGRATION_NAME, LangfusePromptManagement, LangfusePromptManagementError};

// PromptManagementBase.get_chat_completion_prompt's arguments
pub struct ChatCompletionPromptRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<AllMessageValues>,
    pub non_default_params: PromptParams,
    pub prompt_id: Option<&'request str>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
    pub ignore_flags: PromptIgnoreFlags,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChatCompletionPrompt {
    pub model: String,
    pub messages: Vec<AllMessageValues>,
    pub non_default_params: PromptParams,
}

// async_get_chat_completion_prompt's extra arguments: neither litellm_logging_obj nor tools reaches the sync base
pub struct AsyncPromptRequest<'request> {
    pub request: ChatCompletionPromptRequest<'request>,
    pub tools: Option<&'request [Json]>,
}

// PromptManagementBase.get_chat_completion_prompt
pub async fn get_chat_completion_prompt(
    manager: &LangfusePromptManagement,
    request: ChatCompletionPromptRequest<'_>,
) -> Result<ChatCompletionPrompt, LangfusePromptManagementError> {
    let Some(prompt_id) = request.prompt_id else {
        return Ok(unchanged_prompt(request));
    };
    if !should_run_prompt_management(
        manager,
        Some(prompt_id),
        request.prompt_spec,
        request.dynamic_callback_params,
    )
    .await
    {
        return Ok(unchanged_prompt(request));
    }
    let prompt_template = compile_prompt_client(manager, prompt_id, &request).await?;
    let ignore_flags = resolve_ignore_flags(request.prompt_spec, request.ignore_flags);
    let (model, messages, non_default_params) = post_compile_prompt_processing(
        &prompt_template,
        request.messages,
        request.non_default_params,
        request.model.to_owned(),
        &PromptProcessingOptions {
            integration_name: INTEGRATION_NAME.to_owned(),
            ignore_flags,
        },
    );
    Ok(ChatCompletionPrompt {
        model,
        messages,
        non_default_params,
    })
}

// PromptManagementBase.compile_prompt: the helper, then the client messages it appends
async fn compile_prompt_client(
    manager: &LangfusePromptManagement,
    prompt_id: &str,
    request: &ChatCompletionPromptRequest<'_>,
) -> Result<PromptManagementClient, LangfusePromptManagementError> {
    // compile_prompt calls the sync helper without a prompt spec
    let mut prompt_template = compile_prompt_helper(
        manager,
        &PromptCompileRequest {
            prompt_id: Some(prompt_id),
            prompt_spec: None,
            prompt_variables: request.prompt_variables,
            dynamic_callback_params: request.dynamic_callback_params,
            prompt_label: request.prompt_label,
            prompt_version: request.prompt_version,
        },
    )
    .await?;
    let mut completed = prompt_template.prompt_template.clone();
    completed.extend(request.messages.clone());
    prompt_template.completed_messages = Some(completed);
    Ok(prompt_template)
}

// LangfusePromptManagement.async_get_chat_completion_prompt delegates to the sync base
pub async fn async_chat_completion_prompt(
    manager: &LangfusePromptManagement,
    request: AsyncPromptRequest<'_>,
) -> Result<ChatCompletionPrompt, LangfusePromptManagementError> {
    let AsyncPromptRequest { request: inner, .. } = request;
    get_chat_completion_prompt(manager, inner).await
}

fn unchanged_prompt(request: ChatCompletionPromptRequest<'_>) -> ChatCompletionPrompt {
    ChatCompletionPrompt {
        model: request.model.to_owned(),
        messages: request.messages,
        non_default_params: request.non_default_params,
    }
}
