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
    let parsed_prompt = prompt.clone();
    Ok((parsed_prompt, callback))
}
