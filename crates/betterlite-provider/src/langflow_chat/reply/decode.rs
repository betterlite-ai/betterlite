use betterlite_prompts::arguments::json::{Json, Object};

use crate::langflow_chat::error::{LangFlowError, NO_MESSAGE_MESSAGE};

const EXTRACTION_STATUS: u16 = 500;

pub fn decode_content(body: &[u8], status: u16) -> Result<String, LangFlowError> {
    let response: Json = serde_json::from_slice(body).map_err(|source| LangFlowError::Api {
        status_code: status,
        message: format!("LangFlow returned a non-JSON response: {source}"),
    })?;
    content_from_response(&response).ok_or(LangFlowError::Api {
        status_code: EXTRACTION_STATUS,
        message: String::from(NO_MESSAGE_MESSAGE),
    })
}

fn content_from_response(response: &Json) -> Option<String> {
    let fields = response.as_object()?;
    let first_output = first_entry(fields.get("outputs"))?.as_object()?;
    let first_inner = first_entry(first_output.get("outputs"))?.as_object()?;
    let results_text = first_inner
        .get("results")
        .and_then(Json::as_object)
        .and_then(|results| text_of(results.get("message")));
    if results_text.is_some() {
        return results_text;
    }
    let outputs = first_inner.get("outputs").and_then(Json::as_object)?;
    outputs
        .values()
        .find_map(|value| text_of(value.as_object()?.get("message")))
}

fn first_entry(value: Option<&Json>) -> Option<&Json> {
    match value? {
        Json::List(items) => items.first(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

fn text_of(message: Option<&Json>) -> Option<String> {
    message?
        .as_object()?
        .get("text")?
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
