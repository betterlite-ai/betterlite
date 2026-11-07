use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptIgnoreFlags, PromptManagementClient,
    StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;

use crate::bitbucket::error::BitBucketPromptError;
use crate::bitbucket::manager::BitBucketPromptManager;

use super::prompt_orchestration::{
    BoxFuture, ChatCompletionPrompt, ChatCompletionPromptRequest, PromptManagementBackend,
    PromptOrchestrationError, sync_completion_prompt,
};
use super::variables::to_prompt_object;

impl PromptManagementBackend for BitBucketPromptManager {
    type CompileError = BitBucketPromptError;

    fn integration_name(&self) -> &'static str {
        Self::INTEGRATION_NAME
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        Self::should_run_prompt_management(prompt_id)
    }

    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, Self::CompileError>> {
        let variables = request.prompt_variables.map(to_prompt_object);
        let prompt_id = request.prompt_id;
        let prompt_label = request.prompt_label;
        let prompt_version = request.prompt_version;
        Box::pin(async move {
            Self::async_compile_prompt_helper(
                self,
                prompt_id,
                variables.as_ref(),
                prompt_label,
                prompt_version,
            )
            .await
        })
    }

    // LiteLLM's BitBucket sync hook omits the ignore-flag arguments, so the base method sees false.
    fn get_chat_completion_prompt<'backend>(
        &'backend self,
        request: ChatCompletionPromptRequest<'backend>,
    ) -> BoxFuture<
        'backend,
        Result<ChatCompletionPrompt, PromptOrchestrationError<Self::CompileError>>,
    > {
        let without_flags = ChatCompletionPromptRequest {
            ignore_flags: PromptIgnoreFlags::default(),
            ..request
        };
        sync_completion_prompt(self, without_flags)
    }
}
