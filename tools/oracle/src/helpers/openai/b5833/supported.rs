use serde_json::{Value, json};

use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_chat::options::{SupportedParams, get_supported_openai_params};

use super::{Call, capabilities::Answers, config, empty_catalog};

pub(super) fn supported_one(answers: &Answers, call: &Call) -> Result<Value, GroqChatError> {
    let params = get_supported_openai_params(&SupportedParams {
        model: call.model.as_deref().unwrap_or(""),
        config: &config(),
        catalog: &empty_catalog(),
        capabilities: answers,
    })?;
    Ok(json!({"params": params}))
}
