use betterlite_core::message_utilities::thought_signatures::{
    is_gemini_model, strip_signature_from_id, strip_signatures_from_messages,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use super::errors;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    IsGeminiModel {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        custom_llm_provider: Option<String>,
    },
    StripSignatureFromId {
        tool_call_id: String,
        separator: String,
    },
    StripSignaturesFromMessages {
        messages: Vec<Json>,
        separator: String,
    },
}

pub(super) fn run(call: &Call) -> HelperResult {
    match call {
        Call::IsGeminiModel {
            model,
            custom_llm_provider,
        } => Ok(json!(is_gemini_model(
            model.as_deref(),
            custom_llm_provider.as_deref()
        ))),
        Call::StripSignatureFromId {
            tool_call_id,
            separator,
        } => strip_signature_from_id(tool_call_id, separator)
            .map(|cleaned| json!(cleaned))
            .map_err(|error| errors::value_error(error.to_string())),
        Call::StripSignaturesFromMessages {
            messages,
            separator,
        } => strip_signatures_from_messages(messages, separator)
            .map(|cleaned| json!(cleaned))
            .map_err(|error| errors::value_error(error.to_string())),
    }
}
