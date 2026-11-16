use std::path::Path;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_spec::PromptSpec;
use indexmap::IndexMap;

use crate::registry::mutation::prompt_registry_mutation::callbacks::PromptCallbackManager;
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::startup_init::error::InitPromptsError;

mod tests;

pub fn init_prompts(
    registry: &mut InMemoryPromptRegistry,
    all_prompts: &[IndexMap<String, Json>],
    config_file_path: Option<&Path>,
    callbacks: &mut dyn PromptCallbackManager,
) -> Result<Vec<PromptSpec>, InitPromptsError> {
    let mut prompt_list = Vec::with_capacity(all_prompts.len());
    for (index, entry) in all_prompts.iter().enumerate() {
        let value = serde_json::to_value(entry)
            .map_err(|source| InitPromptsError::InvalidPrompt { index, source })?;
        let spec: PromptSpec = serde_json::from_value(value)
            .map_err(|source| InitPromptsError::InvalidPrompt { index, source })?;
        let initialized = registry
            .initialize_prompt(&spec, config_file_path, callbacks)
            .map_err(|source| InitPromptsError::Initialize {
                prompt_id: spec.prompt_id.clone(),
                source,
            })?;
        prompt_list.push(initialized);
    }
    tracing::debug!(prompts = ?prompt_list, "Prompt List:");
    Ok(prompt_list)
}
