use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::PromptSpec;
use indexmap::IndexMap;

use crate::registry::prompt_registry_helpers::{
    default_serve_environment, get_base_prompt_id, prompt_environment_or_default,
    registry_key_for_prompt, spec_version,
};

#[derive(Debug, Default)]
pub struct InMemoryPromptRegistry {
    pub in_memory_prompts: IndexMap<String, PromptSpec>,
    pub prompt_id_to_custom_prompt: IndexMap<String, PromptManagementClient>,
}

impl InMemoryPromptRegistry {
    #[must_use]
    pub fn resolve_prompt_spec(
        &self,
        prompt_id: &str,
        version: Option<i64>,
        environment: Option<&str>,
    ) -> Option<PromptSpec> {
        let base_prompt_id = get_base_prompt_id(prompt_id);
        let base_matches: Vec<&PromptSpec> = self
            .in_memory_prompts
            .values()
            .filter(|spec| get_base_prompt_id(&spec.prompt_id) == base_prompt_id)
            .collect();
        let resolved_environment = environment.map_or_else(
            || default_serve_environment(&base_matches),
            |requested| requested.to_owned(),
        );
        let mut candidates = base_matches.into_iter().filter(|spec| {
            prompt_environment_or_default(Some(spec.environment.as_str())) == resolved_environment
        });
        if let Some(requested_version) = version {
            return candidates
                .find(|spec| spec_version(spec) == requested_version)
                .cloned();
        }
        let mut selected = candidates.next()?;
        for candidate in candidates {
            if spec_version(candidate) > spec_version(selected) {
                selected = candidate;
            }
        }
        Some(selected.clone())
    }

    #[must_use]
    pub fn get_prompt_callback_for_prompt(
        &self,
        prompt: &PromptSpec,
    ) -> Option<PromptManagementClient> {
        let registry_key = registry_key_for_prompt(prompt);
        let callback = self.prompt_id_to_custom_prompt.get(&registry_key)?;
        Some(callback.clone())
    }

    #[must_use]
    pub fn has_config_prompt(&self, base_prompt_id: &str) -> bool {
        self.in_memory_prompts.values().any(|spec| {
            get_base_prompt_id(&spec.prompt_id) == base_prompt_id
                && spec.prompt_info.prompt_type == "config"
        })
    }
}

mod tests;
