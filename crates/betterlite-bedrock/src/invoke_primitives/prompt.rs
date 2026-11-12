use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_logging::analytics::python::repr;
use betterlite_prompts::builtin::{Content, Message};
use betterlite_prompts::cohere::error::CohereError;
use betterlite_prompts::cohere::prompt::cohere_message_pt;
use betterlite_prompts::cohere::types::CohereToolResult;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, RoleTemplate, custom_prompt};
use betterlite_prompts::factory::hub::{ChatTemplate, TokenizerConfig};
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use betterlite_prompts::message_content::Message as ContentMessage;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

use crate::aws_endpoints::provider::BedrockInvokeProvider as Provider;

const LLAMA3_INSTRUCT_MODEL: &str = "meta-llama/Meta-Llama-3-8B-Instruct";
const DEEPSEEK_MODEL: &str = "deepseek-r1/deepseek-r1-7b-instruct";

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error(transparent)]
    Factory(#[from] FactoryError),
    #[error(transparent)]
    Cohere(#[from] CohereError),
    #[error("the {provider} prompt is not text")]
    NotText { provider: &'static str },
    #[error("message {index} cannot be converted to the Cohere prompt shape")]
    Message {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomPromptEntry {
    pub roles: BTreeMap<String, RoleTemplate>,
    #[serde(default)]
    pub initial_prompt_value: String,
    #[serde(default)]
    pub final_prompt_value: String,
}

pub struct PromptConversion<'text> {
    pub model: &'text str,
    pub messages: &'text [Message],
    pub provider: Option<Provider>,
    pub custom_prompt_dict: &'text BTreeMap<String, CustomPromptEntry>,
    pub prompts: &'text PromptFactory,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedPrompt {
    pub prompt: String,
    pub chat_history: Option<Vec<CohereToolResult>>,
}

pub async fn convert_messages_to_prompt(
    conversion: PromptConversion<'_>,
) -> Result<ConvertedPrompt, ConversionError> {
    if let Some(entry) = conversion.custom_prompt_dict.get(conversion.model) {
        let template = CustomPromptTemplate {
            roles: entry.roles.clone(),
            initial_prompt_value: entry.initial_prompt_value.clone(),
            final_prompt_value: entry.final_prompt_value.clone(),
            bos_token: String::new(),
            eos_token: String::new(),
        };
        return Ok(ConvertedPrompt {
            prompt: custom_prompt(&template, conversion.messages)?,
            chat_history: None,
        });
    }
    match conversion.provider {
        Some(
            Provider::Anthropic
            | Provider::Amazon
            | Provider::Mistral
            | Provider::Meta
            | Provider::Llama,
        ) => factory_prompt(&conversion, conversion.model, Some("bedrock"), "bedrock").await,
        Some(Provider::Cohere) => cohere_prompt(conversion.messages),
        Some(Provider::DeepseekR1) => {
            factory_prompt(&conversion, DEEPSEEK_MODEL, None, "deepseek_r1").await
        }
        Some(
            Provider::Ai21
            | Provider::Nova
            | Provider::Qwen3
            | Provider::Qwen2
            | Provider::Twelvelabs
            | Provider::Openai
            | Provider::Stability
            | Provider::Moonshot,
        )
        | None => concatenated(conversion.messages),
    }
}

async fn factory_prompt(
    conversion: &PromptConversion<'_>,
    model: &str,
    custom_provider: Option<&str>,
    operation: &'static str,
) -> Result<ConvertedPrompt, ConversionError> {
    let local_template = match custom_provider {
        Some("bedrock") => bedrock_template(&model.to_lowercase()),
        _ => Some(model),
    };
    if let Some(template) = local_template
        && !has_local_template(conversion.prompts, template)
    {
        return Err(ConversionError::Factory(FactoryError::NoChatTemplate));
    }
    match prompt_factory(
        conversion.prompts,
        model,
        conversion.messages,
        custom_provider,
        conversion.now,
    )
    .await?
    {
        Output::Text(prompt) => Ok(ConvertedPrompt {
            prompt,
            chat_history: None,
        }),
        Output::Ollama(_) | Output::None => Err(ConversionError::NotText {
            provider: operation,
        }),
    }
}

fn cohere_prompt(messages: &[Message]) -> Result<ConvertedPrompt, ConversionError> {
    let mut converted = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let encoded = serde_json::to_string(message)
            .map_err(|source| ConversionError::Message { index, source })?;
        converted.push(
            serde_json::from_str::<ContentMessage>(&encoded)
                .map_err(|source| ConversionError::Message { index, source })?,
        );
    }
    let rendered = cohere_message_pt(&converted)?;
    Ok(ConvertedPrompt {
        prompt: rendered.prompt,
        chat_history: Some(rendered.tool_results),
    })
}

fn concatenated(messages: &[Message]) -> Result<ConvertedPrompt, ConversionError> {
    let mut prompt = String::new();
    for message in messages {
        match &message.content {
            Content::Text(text) => prompt.push_str(text),
            Content::Parts(parts) => prompt.push_str(&repr(parts).map_err(FactoryError::from)?),
            Content::Null => prompt.push_str("None"),
        }
    }
    Ok(ConvertedPrompt {
        prompt,
        chat_history: None,
    })
}

fn bedrock_template(lower: &str) -> Option<&'static str> {
    let matched_earlier = lower.contains("amazon.titan-text")
        || lower.contains("anthropic.")
        || lower.contains("mistral.")
        || (lower.contains("llama2") && lower.contains("chat"));
    let instruct =
        (lower.contains("llama3") || lower.contains("llama4")) && lower.contains("instruct");
    (!matched_earlier && instruct).then_some(LLAMA3_INSTRUCT_MODEL)
}

fn has_local_template(prompts: &PromptFactory, model: &str) -> bool {
    let cached = prompts.cache.lock().unwrap_or_else(PoisonError::into_inner);
    match cached.get(model) {
        Some(TokenizerConfig::Success(Some(document))) => {
            matches!(document.chat_template, ChatTemplate::Text(_))
        }
        Some(TokenizerConfig::Success(None) | TokenizerConfig::Failure) | None => false,
    }
}
