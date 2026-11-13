use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::cohere::history::cohere_messages_pt_v2;
use betterlite_prompts::cohere::types::{ChatHistoryEntry, CohereMessage, CohereToolResult};
use betterlite_prompts::message_content::Message;

pub mod error;
pub mod reply;
pub mod tools;

mod tests;

use error::CohereV1ConfigError;

// LiteLLM advertises these names, and map_openai_params copies only the renames below
pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "stream",
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "stop",
    "n",
    "tools",
    "tool_choice",
    "seed",
    "extra_headers",
];

// LiteLLM's CohereChatConfig constructor declares these settings, and every one starts absent
const SETTINGS: &str = "preamble chat_history generation_id response_id conversation_id prompt_truncation connectors search_queries_only documents temperature max_tokens max_completion_tokens k p frequency_penalty presence_penalty tools tool_results seed";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CohereV1ChatConfig {
    settings: Object,
}

impl CohereV1ChatConfig {
    pub fn with_settings(settings: Object) -> Result<Self, CohereV1ConfigError> {
        if let Some(key) = settings
            .keys()
            .find(|key| !SETTINGS.split_whitespace().any(|name| name == key.as_str()))
        {
            return Err(CohereV1ConfigError::UnknownSetting { key: key.clone() });
        }
        Ok(Self { settings })
    }

    // a null setting is an absent one, as the project's compatibility policy says for every value
    #[must_use]
    pub fn get_config(&self) -> Object {
        self.settings
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        let target = match name.as_str() {
            "stream" | "temperature" | "frequency_penalty" | "presence_penalty" | "tools"
            | "seed" => name.as_str(),
            // both limits write the same key, so the one later in the input wins
            "max_tokens" | "max_completion_tokens" => "max_tokens",
            "n" => "num_generations",
            "top_p" => "p",
            "stop" => "stop_sequences",
            // tool_choice and extra_headers are advertised but this method leaves them out
            _ => continue,
        };
        optional_params.insert(target.to_owned(), value.clone());
    }
    optional_params
}

pub struct RequestInput<'request> {
    pub config: &'request CohereV1ChatConfig,
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
}

pub fn request_options(input: &RequestInput<'_>) -> Result<Object, CohereV1ConfigError> {
    let mut options = input.optional_params.clone();
    for (key, value) in input.config.get_config() {
        options.entry(key).or_insert(value);
    }
    let prompt = cohere_messages_pt_v2(input.messages)?;
    if let Some(tools) = options.get("tools") {
        let translated = tools::cohere_tools(tools)?;
        options.insert("tools".to_owned(), translated);
    }
    match prompt.message {
        CohereMessage::Text(message) => {
            options.insert("message".to_owned(), Json::Text(message));
        }
        CohereMessage::ToolResult(result) => {
            options.insert(
                "tool_results".to_owned(),
                Json::List(vec![tool_result_value(&result)]),
            );
        }
    }
    // the separately built history never enters the request, and a USER tail forces a single step
    if prompt
        .chat_history
        .last()
        .is_some_and(|entry| matches!(entry, ChatHistoryEntry::User { .. }))
    {
        options.insert("force_single_step".to_owned(), Json::Bool(true));
    }
    Ok(options)
}

// the request writes the tool result exactly as LiteLLM's ToolResultObject holds it
fn tool_result_value(result: &CohereToolResult) -> Json {
    let call = Object::from_iter([
        ("name".to_owned(), Json::Text(result.call.name.clone())),
        ("parameters".to_owned(), result.call.parameters.clone()),
    ]);
    Json::Object(Object::from_iter([
        ("call".to_owned(), Json::Object(call)),
        ("outputs".to_owned(), Json::List(result.outputs.clone())),
    ]))
}
