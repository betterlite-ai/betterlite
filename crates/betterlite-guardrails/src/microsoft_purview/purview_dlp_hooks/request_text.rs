use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;

use crate::purview_dlp_evaluation::error_handling::{self, PurviewDlpHttpError};
use crate::purview_dlp_evaluation::responses_input::responses_api_input_to_str;
use crate::purview_graph_client::completion_prompt::{
    completion_prompt_to_str, is_token_id_prompt,
};
use crate::purview_graph_client::message_concat::prompt_text_for_dlp;

use super::Data;
use super::conversion;

pub(super) const TOKEN_ID_ERROR: &str =
    "Microsoft Purview DLP: Token-id completion prompts cannot be scanned for DLP in blocking mode";
pub(super) const MESSAGES_ERROR: &str =
    "Microsoft Purview DLP: messages must be a list of chat messages";

fn messages_text(data: &Data) -> Result<Option<String>, PurviewDlpHttpError> {
    let messages = match data.get("messages") {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Array(messages)) => messages,
        Some(_wrong) => return Err(error_handling::message_error(MESSAGES_ERROR)),
    };
    if messages.is_empty() {
        return Ok(None);
    }
    let mut parsed = Vec::with_capacity(messages.len());
    for message in messages {
        let chat: Message = serde_json::to_string(message)
            .and_then(|encoded| serde_json::from_str(&encoded))
            .map_err(|_source| error_handling::message_error(MESSAGES_ERROR))?;
        parsed.push(chat);
    }
    Ok(prompt_text_for_dlp(&parsed))
}

// LiteLLM rejects token-id completion prompts and raises on failed Responses input only in blocking mode.
pub(super) fn prompt_text(
    data: &Data,
    request_data: &Object,
    call_type: Option<CallType>,
    cache: &DualLocalCache,
    blocking: bool,
) -> Result<Option<String>, PurviewDlpHttpError> {
    match call_type {
        Some(CallType::Responses | CallType::Aresponses) => {
            responses_api_input_to_str(request_data, cache, blocking)
        }
        Some(CallType::TextCompletion | CallType::AtextCompletion) => {
            let prompt = data.get("prompt").map(conversion::to_json);
            if blocking && is_token_id_prompt(prompt.as_ref()) {
                return Err(error_handling::message_error(TOKEN_ID_ERROR));
            }
            Ok(completion_prompt_to_str(prompt.as_ref()))
        }
        _ => messages_text(data),
    }
}
