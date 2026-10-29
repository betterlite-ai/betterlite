use crate::builtin::{self, Message, OllamaPrompt};
use crate::error::FactoryError;
use chrono::{DateTime, Utc};

mod bedrock;
mod bounds;
pub mod custom;
mod generic;
mod llamaguard;
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Text(String),
    Ollama(OllamaPrompt),
    /// `LiteLLM`'s generic branch falls off its if/elif chain with no return for this model shape.
    None,
}

pub fn prompt_factory(
    model: &str,
    messages: &[Message],
    custom_llm_provider: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Output, FactoryError> {
    let lower = model.to_lowercase();
    match custom_llm_provider {
        Some("ollama") => builtin::ollama::ollama(&lower, messages)
            .map(Output::Ollama)
            .map_err(FactoryError::from),
        Some("anthropic") => anthropic(&lower, messages),
        Some("mistral") => Err(FactoryError::Unsupported {
            operation: "mistral_transform",
        }),
        Some("bedrock") => bedrock::dispatch(&lower, messages)
            .unwrap_or_else(|| generic::fallback(&lower, messages, now)),
        Some("clarifai") => {
            clarifai(&lower, messages).unwrap_or_else(|| generic::fallback(&lower, messages, now))
        }
        Some("perplexity") => Err(FactoryError::Unsupported {
            operation: "perplexity",
        }),
        Some("azure_text") => Err(FactoryError::Unsupported {
            operation: "azure_text",
        }),
        Some("watsonx") => Err(FactoryError::Unsupported {
            operation: "watsonx",
        }),
        _ => generic::fallback(&lower, messages, now),
    }
}

fn anthropic(lower: &str, messages: &[Message]) -> Result<Output, FactoryError> {
    if lower == "claude-2" || lower == "claude-instant-1" {
        builtin::claude(messages)
            .map(Output::Text)
            .map_err(FactoryError::from)
    } else {
        Err(FactoryError::Unsupported {
            operation: "anthropic_messages",
        })
    }
}

fn clarifai(lower: &str, messages: &[Message]) -> Option<Result<Output, FactoryError>> {
    lower.contains("claude").then(|| {
        builtin::claude(messages)
            .map(Output::Text)
            .map_err(FactoryError::from)
    })
}
