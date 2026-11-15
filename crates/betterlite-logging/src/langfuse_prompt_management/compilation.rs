use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient,
    StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;

use super::clients::PromptClientRequest;
use super::prompt::{compile_prompt, optional_params_from_langfuse, template_model};
use super::{LangfusePromptManagement, LangfusePromptManagementError};

// should_run_prompt_management: the fetch validates, and a failure sends the caller's own messages
pub async fn should_run_prompt_management(
    manager: &LangfusePromptManagement,
    prompt_id: Option<&str>,
    _prompt_spec: Option<&PromptSpec>,
    dynamic_callback_params: &StandardCallbackDynamicParams,
) -> bool {
    let Some(id) = prompt_id else {
        return false;
    };
    let Ok(request) = PromptClientRequest::from_dynamic_params(dynamic_callback_params) else {
        return false;
    };
    let Ok(client) = manager.clients.client_init(&manager.services, &request) else {
        return false;
    };
    match client.get_prompt(id, None, None).await {
        Ok(_prompt) => true,
        Err(error) => {
            tracing::debug!("Langfuse prompt '{id}' cannot be used: {error}");
            false
        }
    }
}

// LangfusePromptManagement._compile_prompt_helper
pub async fn compile_prompt_helper(
    manager: &LangfusePromptManagement,
    request: &PromptCompileRequest<'_>,
) -> Result<PromptManagementClient, LangfusePromptManagementError> {
    let Some(prompt_id) = request.prompt_id else {
        return Err(LangfusePromptManagementError::MissingPromptId);
    };
    let client_request = PromptClientRequest::from_dynamic_params(request.dynamic_callback_params)?;
    let client = manager
        .clients
        .client_init(&manager.services, &client_request)?;
    let prompt = client
        .get_prompt(prompt_id, request.prompt_label, request.prompt_version)
        .await?;
    let compiled = compile_prompt(&prompt, request.prompt_variables);
    let model = template_model(&prompt)?;
    let optional_params = optional_params_from_langfuse(&prompt)?;
    Ok(PromptManagementClient {
        prompt_id: Some(prompt_id.to_owned()),
        prompt_template: compiled,
        prompt_template_model: model,
        prompt_optional_params: Some(optional_params),
        completed_messages: None,
    })
}

// LangfusePromptManagement.async_compile_prompt_helper: the async path is the sync helper
pub async fn async_compile_prompt_helper(
    manager: &LangfusePromptManagement,
    request: &PromptAsyncCompileRequest<'_>,
) -> Result<PromptManagementClient, LangfusePromptManagementError> {
    let sync_request = PromptCompileRequest {
        prompt_id: request.prompt_id,
        prompt_spec: request.prompt_spec,
        prompt_variables: request.prompt_variables,
        dynamic_callback_params: request.dynamic_callback_params,
        prompt_label: request.prompt_label,
        prompt_version: request.prompt_version,
    };
    compile_prompt_helper(manager, &sync_request).await
}
