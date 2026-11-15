use std::collections::BTreeMap;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, RoleTemplate, custom_prompt};
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::completion::error::CompletionError;

// all three keys are required, as LiteLLM indexes them from litellm.custom_prompt_dict
#[derive(Debug, Clone, Deserialize)]
pub struct CustomPromptEntry {
    pub roles: BTreeMap<String, RoleTemplate>,
    pub initial_prompt_value: String,
    pub final_prompt_value: String,
}

pub struct PromptSource<'source> {
    pub custom_prompt_dict: &'source BTreeMap<String, CustomPromptEntry>,
    pub prompts: &'source PromptFactory,
    pub now: DateTime<Utc>,
}

// transform_request and transform_response each rebuild the prompt the same way
pub async fn prompt_from_messages(
    source: &PromptSource<'_>,
    model: &str,
    messages: &[Message],
) -> Result<String, CompletionError> {
    if let Some(entry) = source.custom_prompt_dict.get(model) {
        let template = CustomPromptTemplate {
            roles: entry.roles.clone(),
            initial_prompt_value: entry.initial_prompt_value.clone(),
            final_prompt_value: entry.final_prompt_value.clone(),
            bos_token: String::new(),
            eos_token: String::new(),
        };
        return custom_prompt(&template, messages)
            .map_err(|error| CompletionError::Prompt(Box::new(error)));
    }
    let output = prompt_factory(
        source.prompts,
        model,
        messages,
        Some("anthropic"),
        source.now,
    )
    .await
    .map_err(|error| CompletionError::Prompt(Box::new(error)))?;
    match output {
        Output::Text(prompt) => Ok(prompt),
        Output::Ollama(_) | Output::None => Err(CompletionError::PromptNotText),
    }
}
