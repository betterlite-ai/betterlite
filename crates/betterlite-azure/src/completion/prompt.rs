use betterlite_prompts::azure::text::azure_text_pt;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use betterlite_prompts::message_content::Message as ContentMessage;
use chrono::{DateTime, Utc};

use super::error::AzureTextError;

pub const PROMPT_NOT_TEXT: &str = "the azure_text prompt is not text";

// prompt_factory(custom_llm_provider="azure_text") sends every call through the azure_text adapter
pub async fn resolve_prompt(
    factory: &PromptFactory,
    model: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<String, AzureTextError> {
    match prompt_factory(factory, model, messages, Some("azure_text"), now).await {
        Ok(Output::Text(prompt)) => Ok(prompt),
        Ok(Output::Ollama(_prompt) | Output::None) => {
            Err(AzureTextError::internal(PROMPT_NOT_TEXT))
        }
        Err(FactoryError::Unsupported {
            operation: "azure_text",
        }) => azure_prompt(messages),
        Err(error) => Err(AzureTextError::internal(error.to_string())),
    }
}

// the azure_text branch's own converter: B182 azure_text_pt over the parsed message records
fn azure_prompt(messages: &[Message]) -> Result<String, AzureTextError> {
    let parsed = serde_json::to_value(messages)
        .and_then(serde_json::from_value::<Vec<ContentMessage>>)
        .map_err(|source| AzureTextError::internal(source.to_string()))?;
    azure_text_pt(&parsed).map_err(|error| AzureTextError::internal(error.to_string()))
}
