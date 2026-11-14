use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::{InputRequest, input_messages};
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;

use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_dlp_evaluation::error_handling::{self, PurviewDlpHttpError};
use crate::purview_graph_client::message_concat::prompt_text_for_dlp;

const TRANSFORM_ERROR: &str = "Microsoft Purview DLP: Responses API input could not be transformed for DLP scanning in blocking mode";

fn convert_json(value: &Json) -> DecisionsJson {
    match value {
        Json::Null => DecisionsJson::Null,
        Json::Bool(flag) => DecisionsJson::Bool(*flag),
        Json::Number(number) => DecisionsJson::Number(number.clone()),
        Json::Text(text) => DecisionsJson::Text(text.clone()),
        Json::List(items) => DecisionsJson::List(items.iter().map(convert_json).collect()),
        Json::Object(fields) => DecisionsJson::Object(
            fields
                .iter()
                .map(|(key, nested)| (key.clone(), convert_json(nested)))
                .collect(),
        ),
    }
}

fn transform_failure(
    reason: &str,
    raise_on_failure: bool,
) -> Result<Option<String>, PurviewDlpHttpError> {
    if raise_on_failure {
        return Err(error_handling::message_error(TRANSFORM_ERROR));
    }
    tracing::warn!(
        reason,
        "Purview DLP: failed to transform responses API input"
    );
    Ok(None)
}

impl MicrosoftPurviewDLPGuardrail {
    pub fn responses_api_input_to_str(
        data: &Object,
        cache: &DualLocalCache,
        raise_on_failure: bool,
    ) -> Result<Option<String>, PurviewDlpHttpError> {
        let input = data
            .get("input")
            .filter(|value| !matches!(value, Json::Null));
        let instructions = data
            .get("instructions")
            .and_then(Json::as_str)
            .map(str::to_owned);
        if input.is_none() && instructions.is_none() {
            return Ok(None);
        }
        let request = InputRequest {
            input: input.map(convert_json),
            instructions,
        };
        let messages = match input_messages(&request, false, ApiLimits::default(), cache) {
            Ok(messages) => messages,
            Err(source) => return transform_failure(&source.to_string(), raise_on_failure),
        };
        // B909's converter emits its JSON message records; they enter the typed chat messages here.
        let parsed: Result<Vec<Message>, serde_json::Error> = messages
            .iter()
            .map(|message| serde_json::from_value(message.clone()))
            .collect();
        let Ok(chat_messages) = parsed else {
            return transform_failure(
                "the transformed messages are not chat messages",
                raise_on_failure,
            );
        };
        Ok(prompt_text_for_dlp(&chat_messages))
    }
}

mod tests;
