use crate::builtin::Message;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalTemplate {
    pub chat_template: String,
    #[serde(default)]
    pub bos_token: Option<TokenValue>,
    #[serde(default)]
    pub eos_token: Option<TokenValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TokenValue {
    Text(String),
    Object(TokenObject),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TokenObject {
    #[serde(default)]
    pub content: Option<String>,
}

pub fn extract_token_value(token: Option<TokenValue>) -> String {
    match token {
        None => String::new(),
        Some(TokenValue::Text(text)) => text,
        Some(TokenValue::Object(object)) => object.content.unwrap_or_default(),
    }
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("{source}")]
    Compile {
        #[source]
        source: minijinja::Error,
    },
    #[error("Error rendering template - {source}")]
    Render {
        #[source]
        source: minijinja::Error,
    },
    #[error("chat template exceeds its byte budget")]
    TemplateTooLarge,
    #[error("rendered output exceeds its byte budget")]
    OutputTooLarge,
}

mod bounds;
mod engine;
mod render;
mod tests;

pub fn hf_chat_template(
    template: &LocalTemplate,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<String, RenderError> {
    bounds::check_template(&template.chat_template)?;
    let env = engine::build(now);
    let bos_token = extract_token_value(template.bos_token.clone());
    let eos_token = extract_token_value(template.eos_token.clone());
    let rendered = render::render_chat_template(
        &env,
        &template.chat_template,
        &bos_token,
        &eos_token,
        messages,
    )?;
    bounds::check_output(rendered)
}
