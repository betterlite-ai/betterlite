use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptIgnoreFlags, PromptManagementClient,
    StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;

use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_prompt_compiler::async_compile;
use crate::gitlab_prompt_compiler::error::CompileError;
use crate::gitlab_prompt_compiler::gate;
use crate::gitlab_prompt_compiler::init::manager::GitLabPromptManager;

use super::prompt_orchestration::{
    BoxFuture, ChatCompletionPrompt, ChatCompletionPromptRequest, PromptManagementBackend,
    PromptOrchestrationError, sync_completion_prompt,
};

impl<Transport> PromptManagementBackend for GitLabPromptManager<Transport>
where
    Transport: GitLabTransport,
{
    type CompileError = Box<CompileError>;

    fn integration_name(&self) -> &'static str {
        Self::INTEGRATION_NAME
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        gate::should_run_prompt_management(prompt_id, prompt_spec, dynamic_callback_params)
    }

    // GitLab's compile helper is async in betterlite, so the sync flow awaits the same helper.
    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, Self::CompileError>> {
        Box::pin(async_compile::async_compile_prompt_helper(self, request))
    }

    // LiteLLM's GitLab sync hook omits the ignore-flag arguments, so the base method sees false.
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
