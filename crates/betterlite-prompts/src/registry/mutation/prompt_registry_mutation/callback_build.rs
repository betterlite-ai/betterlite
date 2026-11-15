use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::PromptSpec;

use super::error::PromptRegistryError;
use super::initializers::prompt_initializer;

pub fn build_prompt_callback(
    prompt: &PromptSpec,
) -> Result<(PromptSpec, PromptManagementClient), PromptRegistryError> {
    let integration = prompt.litellm_params.prompt_integration.as_str();
    let backend = prompt_initializer(integration).ok_or_else(|| {
        PromptRegistryError::UnsupportedIntegration {
            integration: integration.to_owned(),
        }
    })?;
    let callback = backend.initializer(&prompt.litellm_params, prompt);
    let parsed_prompt = PromptSpec {
        prompt_id: prompt.prompt_id.clone(),
        litellm_params: prompt.litellm_params.clone(),
        prompt_info: prompt.prompt_info.clone(),
        created_at: prompt.created_at,
        updated_at: prompt.updated_at,
        version: prompt.version,
        environment: prompt.environment.clone(),
        created_by: prompt.created_by.clone(),
    };
    Ok((parsed_prompt, callback))
}
