use std::collections::BTreeMap;

use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptAsyncCompileRequest, PromptManagementClient,
    StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::arguments::json::Object;
use crate::cache_accounting::dialect::ModelFacts;
use crate::cache_placement::chat::{ChatPrompt, apply_chat_injections};
use crate::message_content::Message;
use crate::orchestration::prompt_orchestration::{
    BoxFuture, ChatCompletionPrompt, ChatCompletionPromptRequest, CompletionFuture,
    PromptManagementBackend, PromptOrchestrationError,
};

use super::INTEGRATION_NAME;
use super::error::CacheControlCallbackError;
use super::result::{compile_prompt_helper, sync_compile_request};

pub struct AnthropicCacheControlHook<Facts> {
    pub environment: BTreeMap<String, String>,
    pub global_api_base: Option<String>,
    pub facts: Facts,
}

impl<Facts> PromptManagementBackend for AnthropicCacheControlHook<Facts>
where
    Facts: ModelFacts + Sync,
{
    type CompileError = CacheControlCallbackError;

    fn integration_name(&self) -> &'static str {
        INTEGRATION_NAME
    }

    fn should_run(
        &self,
        _prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        false
    }

    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, Self::CompileError>> {
        let client = compile_prompt_helper(&sync_compile_request(request));
        Box::pin(async move { Ok(client) })
    }

    fn get_chat_completion_prompt<'backend>(
        &'backend self,
        request: ChatCompletionPromptRequest<'backend>,
    ) -> CompletionFuture<'backend, Self::CompileError> {
        let placed = place_prompt(self, request);
        Box::pin(async move { placed })
    }

    fn async_completion_prompt<'backend>(
        &'backend self,
        request: ChatCompletionPromptRequest<'backend>,
    ) -> CompletionFuture<'backend, Self::CompileError> {
        self.get_chat_completion_prompt(request)
    }
}

fn place_prompt<Facts>(
    hook: &AnthropicCacheControlHook<Facts>,
    request: ChatCompletionPromptRequest<'_>,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<CacheControlCallbackError>>
where
    Facts: ModelFacts,
{
    let ChatCompletionPromptRequest {
        model,
        messages,
        non_default_params,
        prompt_id,
        ..
    } = request;
    let failed = |source| PromptOrchestrationError::Compile {
        prompt_id: prompt_id.unwrap_or("None").to_owned(),
        source,
    };
    let conversation: Vec<Message> = convert(&messages).map_err(failed)?;
    let mut params: Object = convert(&non_default_params).map_err(failed)?;
    let placed = apply_chat_injections(ChatPrompt {
        model,
        messages: &conversation,
        params: &mut params,
        environment: &hook.environment,
        global_api_base: hook.global_api_base.as_deref(),
        facts: &hook.facts,
    })
    .map_err(|source| failed(CacheControlCallbackError::Placement { source }))?;
    let placed_messages: Vec<AllMessageValues> = convert(&placed).map_err(failed)?;
    Ok(ChatCompletionPrompt {
        model: model.to_owned(),
        messages: placed_messages,
        non_default_params: convert(&params).map_err(failed)?,
    })
}

fn convert<Target, Source>(value: &Source) -> Result<Target, CacheControlCallbackError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let encoded =
        serde_json::to_value(value).map_err(|source| CacheControlCallbackError::Wire { source })?;
    serde_json::from_value(encoded).map_err(|source| CacheControlCallbackError::Wire { source })
}
