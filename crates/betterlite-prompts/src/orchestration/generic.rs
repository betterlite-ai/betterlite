use betterlite_models::generic_prompt_api::error::PromptCompileError;
use betterlite_models::generic_prompt_api::manager::GenericPromptManager;
use betterlite_models::generic_prompt_api::request::PromptApiTransport;
use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptManagementClient, StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;

use super::prompt_orchestration::{BoxFuture, PromptManagementBackend};

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
}
