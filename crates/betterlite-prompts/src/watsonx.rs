use std::collections::BTreeMap;
use std::sync::PoisonError;

use chrono::{DateTime, Utc};

use crate::builtin::{self, Message};
use crate::error::FactoryError;
use crate::factory::custom::{CustomPromptTemplate, RoleTemplate, custom_prompt};
use crate::factory::hub::{ChatTemplate, TokenizerConfig};
use crate::factory::{Output, PromptFactory, prompt_factory};
use crate::jinja::{self, LocalTemplate};

mod tests;

// LiteLLM's WatsonXModelPattern strings, tested as substrings of the model in this order
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatsonxModelPattern {
    GraniteChat,
    IbmMistral,
    IbmMistralai,
    GptOss,
    Llama3Instruct,
}

impl WatsonxModelPattern {
    #[must_use]
    pub const fn pattern(self) -> &'static str {
        match self {
            Self::GraniteChat => "granite-chat",
            Self::IbmMistral => "ibm-mistral",
            Self::IbmMistralai => "ibm-mistralai",
            Self::GptOss => "openai/gpt-oss",
            Self::Llama3Instruct => "meta-llama/llama-3-instruct",
        }
    }
}

// The sync and async variants in LiteLLM share one body; betterlite's template call is async, so one function serves both
pub async fn apply_prompt_template(
    factory: &PromptFactory,
    model: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<Option<String>, FactoryError> {
    if model.contains(WatsonxModelPattern::GraniteChat.pattern()) {
        return builtin::granite(messages)
            .map(Some)
            .map_err(FactoryError::from);
    }
    if model.contains(WatsonxModelPattern::IbmMistral.pattern()) {
        return builtin::mistral(messages)
            .map(Some)
            .map_err(FactoryError::from);
    }
    if model.contains(WatsonxModelPattern::GptOss.pattern()) {
        return Ok(hugging_face_template(factory, hugging_face_model(model), messages, now).await);
    }
    if model.contains(WatsonxModelPattern::Llama3Instruct.pattern()) {
        let roles: BTreeMap<String, RoleTemplate> = ["system", "user", "assistant"]
            .into_iter()
            .map(|role| {
                let pre_message = format!("<|start_header_id|>{role}<|end_header_id|>\n");
                let post_message = String::from("<|eot_id|>");
                (
                    role.to_owned(),
                    RoleTemplate {
                        pre_message,
                        post_message,
                    },
                )
            })
            .collect();
        let template = CustomPromptTemplate {
            roles,
            initial_prompt_value: String::from("<|begin_of_text|>"),
            final_prompt_value: String::from("<|start_header_id|>assistant<|end_header_id|>\n"),
            bos_token: String::new(),
            eos_token: String::new(),
        };
        return custom_prompt(&template, messages).map(Some);
    }
    Ok(None)
}

// convert_watsonx_messages_to_prompt: the custom prompt dictionary has no caller at the pin, so it is not a parameter
pub async fn messages_to_prompt(
    factory: &PromptFactory,
    model: &str,
    messages: &[Message],
    provider: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>, FactoryError> {
    if provider == WatsonxModelPattern::IbmMistralai.pattern() {
        return builtin::mistral(messages)
            .map(Some)
            .map_err(FactoryError::from);
    }
    let template = apply_prompt_template(factory, model, messages, now).await?;
    if template.as_ref().is_some_and(|text| !text.is_empty()) {
        return Ok(template);
    }
    // prompt_factory's watsonx branch answers apply_prompt_template for the lowercased model, which is None for no match
    apply_prompt_template(factory, &model.to_lowercase(), messages, now).await
}

fn hugging_face_model(model: &str) -> &str {
    if let Some((_, rest)) = model.rsplit_once("watsonx/") {
        return rest;
    }
    if let Some((_, rest)) = model.rsplit_once("watsonx_text/") {
        return rest;
    }
    model
}

// B215's fetch is private in factory.rs, so a cache miss asks the public generic prompt_factory to fetch it
async fn hugging_face_template(
    factory: &PromptFactory,
    model: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Option<String> {
    if let Some(template) = cached_template(factory, model) {
        return render_template(&template, model, messages, now);
    }
    let fallback = builtin::default_pt(messages).ok();
    let rendered = prompt_factory(factory, model, messages, None, now).await;
    if let Some(template) = cached_template(factory, model) {
        return render_template(&template, model, messages, now);
    }
    // a stored config means the same fetch ran, so prompt_factory's text is the chat_template.jinja render
    if matches!(
        cached_config(factory, model),
        Some(TokenizerConfig::Success(_) | TokenizerConfig::Failure)
    ) {
        let Ok(Output::Text(text)) = rendered else {
            return None;
        };
        if text.is_empty() || fallback.is_some_and(|default| default == text) {
            tracing::debug!(
                "Failed to apply HuggingFace template for model {model}: no template was fetched"
            );
            return None;
        }
        return Some(text);
    }
    // nothing was stored for this model: a generic family answered or the transport failed before caching
    if let Err(error) = rendered {
        tracing::debug!("Failed to apply HuggingFace template for model {model}: {error}");
    } else {
        tracing::debug!(
            "Failed to apply HuggingFace template for model {model}: no template was fetched"
        );
    }
    None
}

fn render_template(
    template: &LocalTemplate,
    model: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Option<String> {
    match jinja::hf_chat_template(template, messages, now) {
        Ok(rendered) if !rendered.is_empty() => Some(rendered),
        Ok(_) => None,
        Err(error) => {
            tracing::debug!("Failed to apply HuggingFace template for model {model}: {error}");
            None
        }
    }
}

fn cached_config(factory: &PromptFactory, model: &str) -> Option<TokenizerConfig> {
    factory
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(model)
        .cloned()
}

fn cached_template(factory: &PromptFactory, model: &str) -> Option<LocalTemplate> {
    let Some(TokenizerConfig::Success(Some(document))) = cached_config(factory, model) else {
        return None;
    };
    let ChatTemplate::Text(chat_template) = document.chat_template else {
        return None;
    };
    Some(LocalTemplate {
        chat_template,
        bos_token: document.bos_token,
        eos_token: document.eos_token,
    })
}
