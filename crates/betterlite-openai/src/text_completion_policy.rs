#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "serde_json test fixtures are concise and type-directed"
    )
)]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "every provider outside the three text-completion families uses the OpenAI config"
)]

pub mod prompt;
pub mod reply;

use betterlite_models::identity::provider_id::ProviderId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::openai_base_config::OpenAiSettings;

mod tests;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextCompletionSettings {
    #[serde(flatten)]
    pub inherited: OpenAiSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_of: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<Value>,
}

pub const SUPPORTED_PARAMS: [&str; 21] = [
    "functions",
    "function_call",
    "temperature",
    "top_p",
    "n",
    "stream",
    "stream_options",
    "stop",
    "max_tokens",
    "presence_penalty",
    "frequency_penalty",
    "logit_bias",
    "user",
    "response_format",
    "seed",
    "tools",
    "tool_choice",
    "max_retries",
    "logprobs",
    "top_logprobs",
    "extra_headers",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextCompletionProviderTag {
    OpenAi,
    FireworksAi,
    TogetherAi,
    TextCompletionInception,
}

#[must_use]
pub const fn select_completion_provider(provider: ProviderId) -> TextCompletionProviderTag {
    match provider {
        ProviderId::FireworksAi => TextCompletionProviderTag::FireworksAi,
        ProviderId::TogetherAi => TextCompletionProviderTag::TogetherAi,
        ProviderId::TextCompletionInception => TextCompletionProviderTag::TextCompletionInception,
        _ => TextCompletionProviderTag::OpenAi,
    }
}

pub fn transform_completion_request(
    model: &str,
    messages: &[Value],
    optional_params: Map<String, Value>,
) -> Result<Map<String, Value>, prompt::PromptError> {
    let prompt_value = prompt::transform_prompt(messages)?;
    let mut request = Map::new();
    request.insert("model".to_owned(), Value::String(model.to_owned()));
    request.insert("prompt".to_owned(), prompt_value);
    request.extend(optional_params);
    Ok(request)
}
