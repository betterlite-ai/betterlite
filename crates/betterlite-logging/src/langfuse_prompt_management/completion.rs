use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptCompileRequest, PromptIgnoreFlags, PromptParams,
    PromptProcessingOptions, PromptSpec, StandardCallbackDynamicParams,
    post_compile_prompt_processing, resolve_ignore_flags,
};

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

// LangfusePromptManagement.async_get_chat_completion_prompt's extra arguments; it forwards neither
// litellm_logging_obj nor tools to the sync base, so the request tools never reach the prompt
pub struct AsyncChatCompletionPromptRequest<'request> {
    pub request: ChatCompletionPromptRequest<'request>,
    pub tools: Option<&'request [Json]>,
}

impl LangfusePromptManagement {
    // PromptManagementBase.get_chat_completion_prompt
    pub async fn get_chat_completion_prompt(
        &self,
        request: ChatCompletionPromptRequest<'_>,
    ) -> Result<ChatCompletionPrompt, LangfusePromptManagementError> {
        let Some(prompt_id) = request.prompt_id else {
            return Ok(unchanged_prompt(request));
        };
        if !self
            .should_run_prompt_management(
                Some(prompt_id),
                request.prompt_spec,
                request.dynamic_callback_params,
            )
            .await
        {
            return Ok(unchanged_prompt(request));
        }
        // PromptManagementBase.compile_prompt calls the sync helper without a prompt spec
        let mut prompt_template = self
            .compile_prompt_helper(&PromptCompileRequest {
                prompt_id: Some(prompt_id),
                prompt_spec: None,
                prompt_variables: request.prompt_variables,
                dynamic_callback_params: request.dynamic_callback_params,
                prompt_label: request.prompt_label,
                prompt_version: request.prompt_version,
            })
            .await?;
        let mut completed = prompt_template.prompt_template.clone();
        completed.extend(request.messages.clone());
        prompt_template.completed_messages = Some(completed);
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

    // LangfusePromptManagement.async_get_chat_completion_prompt delegates to the sync base
    pub async fn async_get_chat_completion_prompt(
        &self,
        request: AsyncChatCompletionPromptRequest<'_>,
    ) -> Result<ChatCompletionPrompt, LangfusePromptManagementError> {
        let AsyncChatCompletionPromptRequest { request, .. } = request;
        self.get_chat_completion_prompt(request).await
    }
}

fn unchanged_prompt(request: ChatCompletionPromptRequest<'_>) -> ChatCompletionPrompt {
    ChatCompletionPrompt {
        model: request.model.to_owned(),
        messages: request.messages,
        non_default_params: request.non_default_params,
    }
}
