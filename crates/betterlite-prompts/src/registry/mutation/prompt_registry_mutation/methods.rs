use std::path::Path;

use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::registry::prompt_registry_helpers::{
    get_base_prompt_id, prompt_environment_or_default, registry_key_for_prompt,
};

use super::callback_build::build_prompt_callback;
use super::callbacks::PromptCallbackManager;
use super::error::PromptRegistryError;

// LiteLLM's methods live on its one class; the type's impl block is another task's file, so callers import this trait
pub trait PromptRegistryMutation {
    fn initialize_prompt(
        &mut self,
        prompt: &PromptSpec,
        _config_file_path: Option<&Path>,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Result<PromptSpec, PromptRegistryError>;

    fn reload_prompt(
        &mut self,
        prompt: &PromptSpec,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Result<PromptSpec, PromptRegistryError>;

    fn sync_prompt_from_db(
        &mut self,
        prompt: &PromptSpec,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Result<PromptSpec, PromptRegistryError>;

    fn remove_prompt(&mut self, registry_key: &str, callbacks: &mut dyn PromptCallbackManager);

    fn delete_prompts_by_base_id(
        &mut self,
        base_prompt_id: &str,
        environment: Option<&str>,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Vec<String>;
}

impl PromptRegistryMutation for InMemoryPromptRegistry {
    // LiteLLM accepts config_file_path here and never reads it at this pin
    fn initialize_prompt(
        &mut self,
        prompt: &PromptSpec,
        _config_file_path: Option<&Path>,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Result<PromptSpec, PromptRegistryError> {
        let registry_key = registry_key_for_prompt(prompt);
        if let Some(existing) = self.in_memory_prompts.get(&registry_key) {
            return Ok(existing.clone());
        }
        let (parsed_prompt, callback) = build_prompt_callback(prompt)?;
        callbacks.add_litellm_callback(&callback);
        self.in_memory_prompts
            .insert(registry_key.clone(), parsed_prompt.clone());
        self.prompt_id_to_custom_prompt
            .insert(registry_key, callback);
        Ok(parsed_prompt)
    }

    // The replacement is built before the stale callback goes away, so a build failure leaves the old registration serving
    fn reload_prompt(
        &mut self,
        prompt: &PromptSpec,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Result<PromptSpec, PromptRegistryError> {
        let (parsed_prompt, new_callback) = build_prompt_callback(prompt)?;
        let registry_key = registry_key_for_prompt(&parsed_prompt);
        let stale_callback = self.prompt_id_to_custom_prompt.shift_remove(&registry_key);
        self.in_memory_prompts.shift_remove(&registry_key);
        if let Some(stale) = stale_callback {
            callbacks.remove_callback_from_all_lists(&stale);
        }
        callbacks.add_litellm_callback(&new_callback);
        self.in_memory_prompts
            .insert(registry_key.clone(), parsed_prompt.clone());
        self.prompt_id_to_custom_prompt
            .insert(registry_key, new_callback);
        Ok(parsed_prompt)
    }

    // LiteLLM skips the reload only when both stored values equal the database row's, never when the row is the same object
    fn sync_prompt_from_db(
        &mut self,
        prompt: &PromptSpec,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Result<PromptSpec, PromptRegistryError> {
        let registry_key = registry_key_for_prompt(prompt);
        let Some(existing) = self.in_memory_prompts.get(&registry_key).cloned() else {
            return self.initialize_prompt(prompt, None, callbacks);
        };
        if same_litellm_params(&existing.litellm_params, &prompt.litellm_params)
            && existing.prompt_info == prompt.prompt_info
        {
            return Ok(existing);
        }
        self.reload_prompt(prompt, callbacks)
    }

    fn remove_prompt(&mut self, registry_key: &str, callbacks: &mut dyn PromptCallbackManager) {
        self.in_memory_prompts.shift_remove(registry_key);
        let stale_callback = self.prompt_id_to_custom_prompt.shift_remove(registry_key);
        if let Some(stale) = stale_callback {
            callbacks.remove_callback_from_all_lists(&stale);
        }
    }

    fn delete_prompts_by_base_id(
        &mut self,
        base_prompt_id: &str,
        environment: Option<&str>,
        callbacks: &mut dyn PromptCallbackManager,
    ) -> Vec<String> {
        let keys_to_delete: Vec<String> = self
            .in_memory_prompts
            .iter()
            .filter(|(_, spec)| {
                get_base_prompt_id(&spec.prompt_id) == base_prompt_id
                    && environment.is_none_or(|environment| {
                        prompt_environment_or_default(Some(spec.environment.as_str()))
                            == environment
                    })
            })
            .map(|(registry_key, _)| registry_key.clone())
            .collect();
        for registry_key in &keys_to_delete {
            self.remove_prompt(registry_key, callbacks);
        }
        keys_to_delete
    }
}

#[must_use]
pub(crate) fn same_litellm_params(left: &PromptLiteLLMParams, right: &PromptLiteLLMParams) -> bool {
    left.prompt_id == right.prompt_id
        && left.prompt_integration == right.prompt_integration
        && left.api_base == right.api_base
        && same_api_key(left.api_key.as_ref(), right.api_key.as_ref())
        && left.provider_specific_query_params == right.provider_specific_query_params
        && left.ignore_model == right.ignore_model
        && left.ignore_optional_params == right.ignore_optional_params
        && left.dotprompt_content == right.dotprompt_content
        && left.extra == right.extra
}

// LiteLLM's == compares the api key strings; the key stays secret, so it is compared in constant time
fn same_api_key(left: Option<&SecretString>, right: Option<&SecretString>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => bool::from(
            left.expose_secret()
                .as_bytes()
                .ct_eq(right.expose_secret().as_bytes()),
        ),
        (None, Some(_)) | (Some(_), None) => false,
    }
}
