use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::OpenAiSettings;
use betterlite_openai::text_completion_policy::TextCompletionSettings;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("the azure text settings cannot be read")]
    Convert(#[from] serde_json::Error),
}

// AzureOpenAITextConfig.__init__: every parameter keeps the None default instead of a class attribute
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureOpenAITextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<Json>,
    #[serde(rename = "n", skip_serializing_if = "Option::is_none")]
    pub number: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Json>,
}

impl AzureOpenAITextConfig {
    // the subclass forwards its eight parameters to OpenAITextCompletionConfig, which stores the non-None ones
    pub fn settings(&self) -> Result<TextCompletionSettings, ConfigError> {
        let converted = serde_json::to_value(self)?;
        let inherited: OpenAiSettings = serde_json::from_value(converted)?;
        Ok(TextCompletionSettings {
            inherited,
            best_of: None,
            echo: None,
            logprobs: None,
            suffix: None,
        })
    }
}
