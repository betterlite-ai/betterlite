use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient,
};

#[must_use]
pub fn compile_prompt_helper(request: &PromptCompileRequest<'_>) -> PromptManagementClient {
    // LiteLLM echoes the requested id and leaves every template and completion field empty
    PromptManagementClient {
        prompt_id: request.prompt_id.map(str::to_owned),
        prompt_template: Vec::new(),
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    }
}

#[must_use]
pub const fn sync_compile_request<'request>(
    request: &PromptAsyncCompileRequest<'request>,
) -> PromptCompileRequest<'request> {
    PromptCompileRequest {
        prompt_id: request.prompt_id,
        prompt_spec: request.prompt_spec,
        prompt_variables: request.prompt_variables,
        dynamic_callback_params: request.dynamic_callback_params,
        prompt_label: request.prompt_label,
        prompt_version: request.prompt_version,
    }
}
