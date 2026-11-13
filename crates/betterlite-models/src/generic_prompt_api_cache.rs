use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use crate::generic_prompt_api::variables::{VariablesError, apply_variables};
use crate::prompt_contracts::{PromptManagementClient, PromptParams};

mod tests;

#[derive(Debug, Default)]
pub struct PromptCache {
    pub(crate) entries: Mutex<HashMap<String, PromptManagementClient>>,
}

impl PromptCache {
    #[must_use]
    pub fn get_cache_key(
        prompt_id: Option<&str>,
        prompt_label: Option<&str>,
        prompt_version: Option<i64>,
    ) -> String {
        let version = prompt_version.map_or_else(|| "None".to_owned(), |number| number.to_string());
        format!(
            "{}:{}:{version}",
            prompt_id.unwrap_or("None"),
            prompt_label.unwrap_or("None")
        )
    }

    pub fn common_caching_logic(
        &self,
        prompt_id: Option<&str>,
        prompt_label: Option<&str>,
        prompt_version: Option<i64>,
        prompt_variables: Option<&PromptParams>,
    ) -> Result<Option<PromptManagementClient>, VariablesError> {
        let cache_key = Self::get_cache_key(prompt_id, prompt_label, prompt_version);
        let cached = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&cache_key)
            .cloned();
        match (cached, prompt_variables) {
            (Some(prompt_client), Some(variables)) if !variables.is_empty() => {
                apply_variables(&prompt_client, variables).map(Some)
            }
            (cached_prompt, _) => Ok(cached_prompt),
        }
    }

    pub fn insert(&self, cache_key: String, prompt_client: PromptManagementClient) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(cache_key, prompt_client);
    }

    pub fn clear_cache(&self) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}
