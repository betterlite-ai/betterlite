use serde_json::{Value, json};

use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_chat::options::{MapParams, map_openai_params};

use super::capabilities::Answers;
use super::{Call, config, empty_catalog};

pub(super) fn options_one(answers: &Answers, call: &Call) -> Result<Value, GroqChatError> {
    let config = config();
    let catalog = empty_catalog();
    let mapped = map_openai_params(&MapParams {
        model: call.model.as_deref().unwrap_or(""),
        non_default: &call.non_default,
        optional_params: &call.optional_params,
        config: &config,
        catalog: &catalog,
        capabilities: answers,
        drop_params: call.drop_params.unwrap_or(false),
    })?;
    Ok(json!({
        "optional_params": mapped.optional_params,
        "non_default": mapped.non_default,
    }))
}
