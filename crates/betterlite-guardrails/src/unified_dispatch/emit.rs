use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_translation::BaseTranslation;
use crate::unified_dispatch::registry::A2A_CALL_TYPES;

#[derive(Debug, Clone, PartialEq)]
pub enum HttpErrorDetail {
    Text(String),
    Object(BTreeMap<String, GuardrailValue>),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}", display_detail(.detail))]
pub struct StreamHttpError {
    pub status_code: u16,
    pub detail: HttpErrorDetail,
}

fn display_detail(detail: &HttpErrorDetail) -> String {
    match detail {
        HttpErrorDetail::Text(text) => text.clone(),
        HttpErrorDetail::Object(fields) => match serde_json::to_string(fields) {
            Ok(written) => written,
            Err(source) => format!("detail could not be written as JSON: {source}"),
        },
    }
}

#[must_use]
pub fn a2a_request_id(
    responses_so_far: &[GuardrailValue],
    request_data: &BTreeMap<String, GuardrailValue>,
) -> GuardrailValue {
    for item in responses_so_far {
        if let GuardrailValue::Object(fields) = item {
            if let Some(id) = fields.get("id") {
                return id.clone();
            }
            continue;
        }
        let GuardrailValue::Text(frame) = item else {
            continue;
        };
        let Ok(GuardrailValue::Object(fields)) =
            serde_json::from_str::<GuardrailValue>(frame.trim())
        else {
            continue;
        };
        if let Some(id) = fields.get("id") {
            return id.clone();
        }
    }
    let body = request_data.get("body").filter(|value| has_content(value));
    let chosen = body.or_else(|| request_data.get("data").filter(|value| has_content(value)));
    match chosen {
        Some(GuardrailValue::Object(fields)) => {
            fields.get("id").cloned().unwrap_or(GuardrailValue::Null)
        }
        Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_),
        )
        | None => GuardrailValue::Null,
    }
}

// LiteLLM falls through `request_data.get("body") or request_data.get("data")`, so an absent or empty body uses data
fn has_content(value: &GuardrailValue) -> bool {
    match value {
        GuardrailValue::Null => false,
        GuardrailValue::Object(fields) => !fields.is_empty(),
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => true,
    }
}

#[must_use]
pub fn a2a_jsonrpc_error_chunk(
    error: &StreamHttpError,
    request_id: GuardrailValue,
) -> GuardrailValue {
    let (message, data) = match &error.detail {
        HttpErrorDetail::Text(text) => (GuardrailValue::Text(text.clone()), BTreeMap::new()),
        HttpErrorDetail::Object(detail) => {
            let message = detail
                .get("error")
                .or_else(|| detail.get("message"))
                .cloned()
                .unwrap_or_else(|| {
                    let written = serde_json::to_string(detail).unwrap_or_else(|source| {
                        format!("detail could not be written as JSON: {source}")
                    });
                    GuardrailValue::Text(written)
                });
            let mut data = BTreeMap::new();
            for (key, value) in detail {
                if key == "error" || key == "message" {
                    continue;
                }
                data.insert(key.clone(), value.clone());
            }
            (message, data)
        }
    };
    let mut error_fields = BTreeMap::new();
    error_fields.insert("code".to_owned(), GuardrailValue::SignedInteger(-32603));
    error_fields.insert("message".to_owned(), message);
    error_fields.insert("data".to_owned(), GuardrailValue::Object(data));
    let mut chunk = BTreeMap::new();
    chunk.insert("jsonrpc".to_owned(), GuardrailValue::Text("2.0".to_owned()));
    chunk.insert("id".to_owned(), request_id);
    chunk.insert("error".to_owned(), GuardrailValue::Object(error_fields));
    GuardrailValue::Object(chunk)
}

pub fn handle_streaming_block<T>(
    translation: &T,
    failure: GuardrailFailure,
    stream_started: bool,
    responses_so_far: &[GuardrailValue],
) -> Result<Vec<Vec<u8>>, GuardrailFailure>
where
    T: BaseTranslation<Failure = GuardrailFailure> + ?Sized,
{
    translation
        .build_block_sse_chunks(&failure, stream_started, Some(responses_so_far))
        .map_or_else(|| Err(failure), Ok)
}

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors LiteLLM's emit_streaming_http_error signature"
)]
pub fn emit_streaming_http_error<T>(
    call_type: Option<CallType>,
    error: StreamHttpError,
    responses_so_far: &[GuardrailValue],
    request_data: &BTreeMap<String, GuardrailValue>,
    translation: Option<&T>,
    stream_started: bool,
    responses_yielded: Option<&[GuardrailValue]>,
) -> Result<Vec<GuardrailValue>, StreamHttpError>
where
    T: BaseTranslation<Failure = GuardrailFailure> + ?Sized,
{
    if call_type.is_some_and(|candidate| A2A_CALL_TYPES.contains(&candidate)) {
        let request_id = a2a_request_id(responses_so_far, request_data);
        return Ok(vec![a2a_jsonrpc_error_chunk(&error, request_id)]);
    }
    if stream_started && let Some(handler) = translation {
        let failure = match &error.detail {
            HttpErrorDetail::Text(text) => GuardrailFailure::Http {
                status: error.status_code,
                detail: text.clone(),
            },
            HttpErrorDetail::Object(detail) => GuardrailFailure::HttpObject {
                status: error.status_code,
                detail: detail
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            },
        };
        if let Some(items) = handler.build_stream_error_items(&failure, responses_yielded) {
            return Ok(items);
        }
    }
    Err(error)
}
