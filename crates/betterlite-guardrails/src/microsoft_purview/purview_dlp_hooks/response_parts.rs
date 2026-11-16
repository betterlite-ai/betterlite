use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use crate::purview_dlp_evaluation::completion_text::{
    CompletionResponse, completion_response_text_parts,
};
use crate::purview_dlp_evaluation::error_handling::{self, PurviewDlpHttpError};

use super::conversion;

const TEXT_COMPLETION_OBJECT: &str = "text_completion";
const RESPONSES_OBJECT: &str = "response";
pub(super) const RESPONSE_ERROR: &str =
    "Microsoft Purview DLP: response is not a valid Responses API response";

// A record tells its class like LiteLLM's isinstance chain does; a Responses body may leave object out.
fn responses_body(fields: &BTreeMap<String, GuardrailValue>) -> bool {
    match fields.get("object") {
        Some(GuardrailValue::Text(kind)) => kind.as_str() == RESPONSES_OBJECT,
        Some(GuardrailValue::Null) | None => {
            matches!(fields.get("output"), Some(GuardrailValue::Array(_)))
        }
        Some(_other) => false,
    }
}

pub(super) fn parts(response: &GuardrailValue) -> Result<Vec<String>, PurviewDlpHttpError> {
    let GuardrailValue::Object(fields) = response else {
        return Ok(Vec::new());
    };
    let object = conversion::object(fields);
    match fields.get("object") {
        Some(GuardrailValue::Text(kind)) if kind.as_str() == TEXT_COMPLETION_OBJECT => Ok(
            completion_response_text_parts(&CompletionResponse::TextCompletion(&object)),
        ),
        _ if responses_body(fields) => {
            let parsed: ResponsesApiResponse = serde_json::to_string(response)
                .and_then(|encoded| serde_json::from_str(&encoded))
                .map_err(|_source| error_handling::message_error(RESPONSE_ERROR))?;
            Ok(completion_response_text_parts(
                &CompletionResponse::Responses(&parsed),
            ))
        }
        _ => Ok(completion_response_text_parts(&CompletionResponse::Model(
            &object,
        ))),
    }
}
