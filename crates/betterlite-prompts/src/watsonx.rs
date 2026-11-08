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
        return jinja::hf_chat_template(&template, messages, now)
            .ok()
            .filter(|rendered| !rendered.is_empty());
    }
    let fallback = builtin::default_pt(messages).ok();
    let Ok(Output::Text(rendered)) = prompt_factory(factory, model, messages, None, now).await
    else {
        return None;
    };
    // the generic branch answers default_pt when the fetch fails, and LiteLLM's branch gives None on any template error
    if rendered.is_empty() || fallback.is_some_and(|default| default == rendered) {
        return None;
    }
    Some(rendered)
}

fn cached_template(factory: &PromptFactory, model: &str) -> Option<LocalTemplate> {
    let cached = factory
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(model)
        .cloned()?;
    let TokenizerConfig::Success(Some(document)) = cached else {
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
