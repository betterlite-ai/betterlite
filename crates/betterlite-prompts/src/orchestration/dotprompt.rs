use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptManagementClient, StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;

use crate::dotprompt_compilation::compile::CompileError;
use crate::dotprompt_compilation::init::manager::DotpromptManager;

use super::prompt_orchestration::{BoxFuture, PromptManagementBackend};
use super::variables::to_prompt_object;

impl PromptManagementBackend for DotpromptManager {
    type CompileError = Box<CompileError>;

    fn integration_name(&self) -> &'static str {
        Self::INTEGRATION_NAME
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        Self::should_run_prompt_management(self, prompt_id)
    }

    fn compile_template<'backend>(
        &'backend self,
        request: &'backend PromptAsyncCompileRequest<'_>,
    ) -> BoxFuture<'backend, Result<PromptManagementClient, Self::CompileError>> {
        let variables = request.prompt_variables.map(to_prompt_object);
        let prompt_id = request.prompt_id;
        let prompt_version = request.prompt_version;
        Box::pin(async move {
            Self::async_compile_prompt_helper(self, prompt_id, variables.as_ref(), prompt_version)
                .await
        })
    }
}
