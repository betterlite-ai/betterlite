use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use crate::builtin::{self, Message, OllamaPrompt};
use crate::error::FactoryError;
use crate::jinja::{self, LocalTemplate, TokenValue};
use chrono::{DateTime, Utc};

use hub::{ChatTemplate, ChatTemplateFile, Hub, TokenizerConfig, TokenizerDocument};

mod bedrock;
mod bounds;
pub mod custom;
mod generic;
pub mod hub;
mod llamaguard;
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Text(String),
    Ollama(OllamaPrompt),
    // LiteLLM's generic branch falls off its if/elif chain with no return for this model shape.
    None,
}

pub struct PromptFactory {
    pub cache: Mutex<BTreeMap<String, TokenizerConfig>>,
    pub hub: Hub,
}

impl PromptFactory {
    pub(crate) async fn hf_chat_template(
        &self,
        model: &str,
        messages: &[Message],
        now: DateTime<Utc>,
    ) -> Result<String, FactoryError> {
        let (template, bos_token, eos_token) = self.template(model).await?;
        let ChatTemplate::Text(chat_template) = template else {
            return Err(FactoryError::TemplateNotText);
        };
        let local = LocalTemplate {
            chat_template,
            bos_token,
            eos_token,
        };
        jinja::hf_chat_template(&local, messages, now).map_err(FactoryError::from)
    }

    async fn cached_tokenizer_config(&self, model: &str) -> Result<TokenizerConfig, FactoryError> {
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(model)
            .cloned();
        if let Some(config) = cached {
            return Ok(config);
        }
        let fetched = self.hub.tokenizer_config(model).await?;
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            // LiteLLM's cache grows with every model it sees; the cap keeps a gateway's memory bounded
            if cache.len() < hub::BUILTIN_ENTRIES + hub::MAX_FETCHED_ENTRIES {
                cache.insert(model.to_owned(), fetched.clone());
            }
        }
        Ok(fetched)
    }

    async fn template(
        &self,
        model: &str,
    ) -> Result<(ChatTemplate, Option<TokenValue>, Option<TokenValue>), FactoryError> {
        let config = self.cached_tokenizer_config(model).await?;
        let TokenizerConfig::Success(document) = config else {
            return self.template_file(model, None).await;
        };
        let Some(tokenizer) = document else {
            return self.template_file(model, None).await;
        };
        if tokenizer.chat_template == ChatTemplate::Absent {
            return self.template_file(model, Some(tokenizer)).await;
        }
        Ok((
            tokenizer.chat_template,
            tokenizer.bos_token,
            tokenizer.eos_token,
        ))
    }

    async fn template_file(
        &self,
        model: &str,
        config: Option<TokenizerDocument>,
    ) -> Result<(ChatTemplate, Option<TokenValue>, Option<TokenValue>), FactoryError> {
        let ChatTemplateFile::Success(chat_template) = self.hub.chat_template_file(model).await
        else {
            return Err(FactoryError::NoChatTemplate);
        };
        let (bos_token, eos_token) = config.map_or((None, None), |document| {
            (document.bos_token, document.eos_token)
        });
        Ok((ChatTemplate::Text(chat_template), bos_token, eos_token))
    }
}

pub async fn prompt_factory(
    factory: &PromptFactory,
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
        Some("bedrock") => match bedrock::dispatch(factory, &lower, messages, now).await {
            Some(result) => result,
            None => generic::fallback(factory, &lower, model, messages, now).await,
        },
        Some("clarifai") => match clarifai(&lower, messages) {
            Some(result) => result,
            None => generic::fallback(factory, &lower, model, messages, now).await,
        },
        Some("perplexity") => Err(FactoryError::Unsupported {
            operation: "perplexity",
        }),
        Some("azure_text") => Err(FactoryError::Unsupported {
            operation: "azure_text",
        }),
        Some("watsonx") => Err(FactoryError::Unsupported {
            operation: "watsonx",
        }),
        _ => generic::fallback(factory, &lower, model, messages, now).await,
    }
}

fn anthropic(lower: &str, messages: &[Message]) -> Result<Output, FactoryError> {
    if lower != "claude-2" && lower != "claude-instant-1" {
        return Err(FactoryError::Unsupported {
            operation: "anthropic_messages",
        });
    }
    Ok(Output::Text(builtin::families::anthropic(
        messages,
        builtin::families::Style::Claude,
    )?))
}

fn clarifai(lower: &str, messages: &[Message]) -> Option<Result<Output, FactoryError>> {
    lower.contains("claude").then(|| {
        let prompt = builtin::families::anthropic(messages, builtin::families::Style::Claude)?;
        Ok(Output::Text(prompt))
    })
}
