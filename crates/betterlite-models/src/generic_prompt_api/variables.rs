use thiserror::Error;

use crate::decisions::Json;
use crate::prompt_contracts::{PromptManagementClient, PromptParams};

#[derive(Debug, Error)]
#[error("prompt variable '{key}' cannot be written as text: {source}")]
pub struct VariablesError {
    pub key: String,
    #[source]
    pub source: serde_json::Error,
}

fn variable_text(key: &str, value: &Json) -> Result<String, VariablesError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).map_err(|source| VariablesError {
                key: key.to_owned(),
                source,
            })
        }
    }
}

// LiteLLM replaces the single-brace form first, so a double-brace placeholder ends up as {value}
pub fn apply_variables(
    prompt_client: &PromptManagementClient,
    variables: &PromptParams,
) -> Result<PromptManagementClient, VariablesError> {
    let mut replacements = Vec::new();
    for (key, value) in variables {
        let text = variable_text(key, value)?;
        replacements.push((format!("{{{key}}}"), format!("{{{{{key}}}}}"), text));
    }
    let mut updated_messages = prompt_client.prompt_template.clone();
    for message in &mut updated_messages {
        let Json::Object(fields) = message else {
            continue;
        };
        let Some(Json::Text(content)) = fields.get_mut("content") else {
            continue;
        };
        for (single, double, text) in &replacements {
            *content = content.replace(single, text).replace(double, text);
        }
    }
    Ok(PromptManagementClient {
        prompt_id: prompt_client.prompt_id.clone(),
        prompt_template: updated_messages,
        prompt_template_model: prompt_client.prompt_template_model.clone(),
        prompt_optional_params: prompt_client.prompt_optional_params.clone(),
        completed_messages: None,
    })
}
