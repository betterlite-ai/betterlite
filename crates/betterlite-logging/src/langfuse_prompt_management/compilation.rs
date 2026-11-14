use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient, PromptSpec,
    StandardCallbackDynamicParams,
};

use super::clients::PromptClientRequest;
use super::prompt::{compile_prompt, optional_params_from_langfuse, template_model};
use super::{LangfusePromptManagement, LangfusePromptManagementError};

impl LangfusePromptManagement {
    // LangfusePromptManagement.should_run_prompt_management: the fetch is the validation, and its
    // failure only means this request goes to the model with the caller's own messages
    pub async fn should_run_prompt_management(
        &self,
        prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        let Some(prompt_id) = prompt_id else {
            return false;
        };
        let Ok(request) = PromptClientRequest::from_dynamic_params(dynamic_callback_params) else {
            return false;
        };
        let Ok(client) = self.clients.client_init(&self.services, &request) else {
            return false;
        };
        match client.get_prompt(prompt_id, None, None).await {
            Ok(_prompt) => true,
            Err(error) => {
                tracing::debug!("Langfuse prompt '{prompt_id}' cannot be used: {error}");
                false
            }
        }
    }

    // LangfusePromptManagement._compile_prompt_helper
    pub async fn compile_prompt_helper(
        &self,
        request: &PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, LangfusePromptManagementError> {
        let Some(prompt_id) = request.prompt_id else {
            return Err(LangfusePromptManagementError::MissingPromptId);
        };
        let client_request =
            PromptClientRequest::from_dynamic_params(request.dynamic_callback_params)?;
        let client = self.clients.client_init(&self.services, &client_request)?;
        let prompt = client
            .get_prompt(prompt_id, request.prompt_label, request.prompt_version)
            .await?;
        let compiled = compile_prompt(&prompt, request.prompt_variables)?;
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
        &self,
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
        self.compile_prompt_helper(&sync_request).await
    }
}
