use betterlite_models::generic_prompt_api::error::PromptCompileError;
use betterlite_models::generic_prompt_api::manager::GenericPromptManager;
use betterlite_models::generic_prompt_api::request::PromptApiTransport;
use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient,
    StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;

use super::prompt_orchestration::{
    BoxFuture, CompileFuture, CompilePromptCall, PromptManagementBackend, complete_compiled_prompt,
};

impl<Transport> PromptManagementBackend for GenericPromptManager<Transport>
where
    Transport: PromptApiTransport,
{
    type CompileError = PromptCompileError<Transport::Error>;

    fn integration_name(&self) -> &'static str {
        Self::INTEGRATION_NAME
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        Self::should_run_prompt_management(prompt_id, prompt_spec)
    }

    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, Self::CompileError>> {
        Box::pin(Self::async_compile_prompt_helper(self, request))
    }

    // LiteLLM's sync generic fetch always reads the manager's own provider-specific query params.
    fn compile_prompt<'backend>(
        &'backend self,
        call: CompilePromptCall<'backend>,
    ) -> CompileFuture<'backend, Self::CompileError> {
        Box::pin(async move {
            let request = PromptCompileRequest {
                prompt_id: Some(call.prompt_id),
                prompt_spec: call.prompt_spec,
                prompt_variables: call.prompt_variables,
                dynamic_callback_params: call.dynamic_callback_params,
                prompt_label: call.prompt_label,
                prompt_version: call.prompt_version,
            };
            let compiled = self.compile_prompt_helper(&request);
            complete_compiled_prompt(call.prompt_id, compiled, call.client_messages)
        })
    }
}
