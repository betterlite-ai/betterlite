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

use crate::aws_endpoints::provider::BedrockInvokeProvider;

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
    pub initial_prompt_value: Option<String>,
    #[serde(default)]
    pub final_prompt_value: Option<String>,
}

pub struct PromptConversion<'text> {
    pub model: &'text str,
    pub messages: &'text [Message],
    pub provider: Option<BedrockInvokeProvider>,
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
        return custom_entry_prompt(entry, &conversion);
    }
    match conversion.provider {
        Some(
            BedrockInvokeProvider::Anthropic
            | BedrockInvokeProvider::Amazon
            | BedrockInvokeProvider::Mistral
            | BedrockInvokeProvider::Meta
            | BedrockInvokeProvider::Llama,
        ) => family_prompt(&conversion).await,
        Some(BedrockInvokeProvider::Cohere) => cohere_prompt(conversion.messages),
        Some(BedrockInvokeProvider::DeepseekR1) => deepseek_prompt(&conversion).await,
        Some(
            BedrockInvokeProvider::Ai21
            | BedrockInvokeProvider::Nova
            | BedrockInvokeProvider::Qwen3
            | BedrockInvokeProvider::Qwen2
            | BedrockInvokeProvider::Twelvelabs
            | BedrockInvokeProvider::Openai
            | BedrockInvokeProvider::Stability
            | BedrockInvokeProvider::Moonshot,
        )
        | None => Ok(ConvertedPrompt {
            prompt: concatenated(conversion.messages)?,
            chat_history: None,
        }),
    }
}

fn custom_entry_prompt(
    entry: &CustomPromptEntry,
    conversion: &PromptConversion<'_>,
) -> Result<ConvertedPrompt, ConversionError> {
    let template = CustomPromptTemplate {
        roles: entry.roles.clone(),
        initial_prompt_value: entry.initial_prompt_value.clone().unwrap_or_default(),
        final_prompt_value: entry.final_prompt_value.clone().unwrap_or_default(),
        bos_token: String::new(),
        eos_token: String::new(),
    };
    Ok(ConvertedPrompt {
        prompt: custom_prompt(&template, conversion.messages)?,
        chat_history: None,
    })
}

async fn family_prompt(
    conversion: &PromptConversion<'_>,
) -> Result<ConvertedPrompt, ConversionError> {
    let lower = conversion.model.to_lowercase();
    if let Some(model) = bedrock_template(&lower)
        && !has_local_template(conversion.prompts, model)
    {
        return Err(ConversionError::Factory(FactoryError::NoChatTemplate));
    }
    match prompt_factory(
        conversion.prompts,
        conversion.model,
        conversion.messages,
        Some("bedrock"),
        conversion.now,
    )
    .await?
    {
        Output::Text(prompt) => Ok(ConvertedPrompt {
            prompt,
            chat_history: None,
        }),
        Output::Ollama(_) | Output::None => Err(ConversionError::NotText {
            provider: "bedrock",
        }),
    }
}

async fn deepseek_prompt(
    conversion: &PromptConversion<'_>,
) -> Result<ConvertedPrompt, ConversionError> {
    if !has_local_template(conversion.prompts, DEEPSEEK_MODEL) {
        return Err(ConversionError::Factory(FactoryError::NoChatTemplate));
    }
    match prompt_factory(
        conversion.prompts,
        DEEPSEEK_MODEL,
        conversion.messages,
        None,
        conversion.now,
    )
    .await?
    {
        Output::Text(prompt) => Ok(ConvertedPrompt {
            prompt,
            chat_history: None,
        }),
        Output::Ollama(_) | Output::None => Err(ConversionError::NotText {
            provider: "deepseek_r1",
        }),
    }
}

fn cohere_prompt(messages: &[Message]) -> Result<ConvertedPrompt, ConversionError> {
    let converted = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let encoded = serde_json::to_string(message)
                .map_err(|source| ConversionError::Message { index, source })?;
            serde_json::from_str::<ContentMessage>(&encoded)
                .map_err(|source| ConversionError::Message { index, source })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rendered = cohere_message_pt(&converted)?;
    Ok(ConvertedPrompt {
        prompt: rendered.prompt,
        chat_history: Some(rendered.tool_results),
    })
}

fn concatenated(messages: &[Message]) -> Result<String, ConversionError> {
    let mut prompt = String::new();
    for message in messages {
        match &message.content {
            Content::Text(text) => prompt.push_str(text),
            Content::Parts(parts) => prompt.push_str(&repr(parts).map_err(FactoryError::from)?),
            Content::Null => prompt.push_str("None"),
        }
    }
    Ok(prompt)
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
