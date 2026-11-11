use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::json;

const FIRST_CHARS: usize = 100;
const SUCCESS_STATUS: &str = "success";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomPromptTemplate {
    pub roles: Json,
    pub initial_prompt_value: String,
    pub final_prompt_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownTokenizerConfig {
    pub tokenizer: Json,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplateRegistry {
    pub custom_prompt_dict: BTreeMap<String, CustomPromptTemplate>,
    pub known_tokenizer_config: BTreeMap<String, KnownTokenizerConfig>,
}

pub fn register_prompt_template<'registry>(
    registry: &'registry mut PromptTemplateRegistry,
    model: &str,
    resolved_model: Option<&str>,
    template: &CustomPromptTemplate,
    tokenizer_config: Option<&Json>,
) -> &'registry BTreeMap<String, CustomPromptTemplate> {
    let mut names = vec![model];
    if let Some(resolved) = resolved_model {
        names.push(resolved);
    }
    if let Some(config) = tokenizer_config.filter(|config| json::counts_as_present(config)) {
        let known = KnownTokenizerConfig {
            tokenizer: config.clone(),
            status: String::from(SUCCESS_STATUS),
        };
        for name in names {
            registry
                .known_tokenizer_config
                .insert(name.to_owned(), known.clone());
        }
    } else {
        for name in names {
            registry
                .custom_prompt_dict
                .insert(name.to_owned(), template.clone());
        }
    }
    &registry.custom_prompt_dict
}

// the text only reaches an error message, so it is standard JSON rather than Python's str() of the list
#[must_use]
pub fn get_first_chars_messages(kwargs: &Json) -> String {
    let Some(fields) = json::object(kwargs) else {
        return String::new();
    };
    let messages = fields.get("messages").unwrap_or(&Json::Null);
    match serde_json::to_string(messages) {
        Ok(text) => text.chars().take(FIRST_CHARS).collect(),
        Err(_error) => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<Json>,
    pub completion_tokens_details: Option<Json>,
}

pub const EMPTY_USAGE: TokenUsage = TokenUsage {
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    prompt_tokens_details: None,
    completion_tokens_details: None,
};

impl TokenUsage {
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut fields = IndexMap::new();
        fields.insert(
            String::from("completion_tokens"),
            Json::Number(serde_json::Number::from(self.completion_tokens)),
        );
        fields.insert(
            String::from("prompt_tokens"),
            Json::Number(serde_json::Number::from(self.prompt_tokens)),
        );
        fields.insert(
            String::from("total_tokens"),
            Json::Number(serde_json::Number::from(self.total_tokens)),
        );
        fields.insert(
            String::from("completion_tokens_details"),
            self.completion_tokens_details.clone().unwrap_or(Json::Null),
        );
        fields.insert(
            String::from("prompt_tokens_details"),
            self.prompt_tokens_details.clone().unwrap_or(Json::Null),
        );
        Json::Object(fields)
    }
}

#[must_use]
pub fn should_run_mock_completion(
    mock_response: &Json,
    mock_tool_calls: &Json,
    mock_timeout: &Json,
) -> bool {
    [mock_response, mock_tool_calls, mock_timeout]
        .iter()
        .any(|value| json::counts_as_present(value))
}
