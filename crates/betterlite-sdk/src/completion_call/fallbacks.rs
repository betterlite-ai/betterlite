use super::error::CompletionCallError;
use super::request::args_to_map;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;
use crate::completion_request::call::FallbackCall;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use uuid::Uuid;
pub const ATTEMPTED_FALLBACKS_HEADER: &str = "x-litellm-attempted-fallbacks";
const HIDDEN_PARAMS: &str = "_hidden_params";
const ADDITIONAL_HEADERS: &str = "additional_headers";
// filter_internal_params' three names (B2204)
const INTERNAL_PARAMS: [&str; 3] = [
    "skip_mcp_handler",
    "mcp_handler_context",
    "_skip_mcp_handler",
];
pub async fn run_fallbacks(
    requester: &dyn CallTarget,
    call: FallbackCall,
    logging: Option<&LoggingHandle>,
) -> Result<CallOutcome, CompletionCallError> {
    let original = call.args.model.clone().unwrap_or_default();
    let mut base = args_to_map(&call.args)?;
    base.shift_remove("model");
    base.shift_remove("acompletion");
    base.shift_remove("fallbacks");
    base.insert(
        String::from("litellm_call_id"),
        Json::Text(Uuid::new_v4().to_string()),
    );
    let base = filter_internal_params(base);
    let mut candidates = Vec::with_capacity(call.fallbacks.len() + 1);
    candidates.push(Json::Text(original.clone()));
    candidates.extend(call.fallbacks);
    let mut last_error: Option<String> = None;
    for (attempted, candidate) in candidates.iter().enumerate() {
        let (model, attempt) = match candidate {
            Json::Text(model) => (model.clone(), base.clone()),
            Json::Object(fields) => {
                let model = fields
                    .get("model")
                    .cloned()
                    .map_or_else(|| Ok(original.clone()), model_name)?;
                (model, overlay(base.clone(), fields))
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                return Err(CompletionCallError::Field { field: "fallbacks" });
            }
        };
        let mut attempt = filter_internal_params(attempt);
        attempt.insert(String::from("model"), Json::Text(model.clone()));
        match requester.call(attempt, logging).await {
            Ok(outcome) if !is_empty(&outcome) => {
                return Ok(with_attempt_header(outcome, attempted));
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(model = %model, error = %error, "Fallback attempt failed");
                last_error = Some(error.to_string());
            }
        }
    }
    Err(CompletionCallError::AllFallbacksFailed {
        last: last_error.unwrap_or_else(|| String::from("None")),
    })
}
fn overlay(mut base: RequestMap, fields: &IndexMap<String, Json>) -> RequestMap {
    for (name, value) in fields {
        base.insert(name.clone(), value.clone());
    }
    base
}
fn model_name(value: Json) -> Result<String, CompletionCallError> {
    match value {
        Json::Text(name) => Ok(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(CompletionCallError::Field { field: "model" })
        }
    }
}
fn filter_internal_params(mut request: RequestMap) -> RequestMap {
    for name in INTERNAL_PARAMS {
        request.shift_remove(name);
    }
    request
}
#[must_use]
pub fn is_empty(outcome: &CallOutcome) -> bool {
    matches!(outcome, CallOutcome::Response(Json::Null))
}
// add_fallback_headers_to_response: the zero-based attempt number under _hidden_params.additional_headers
fn with_attempt_header(outcome: CallOutcome, attempted: usize) -> CallOutcome {
    let value = Json::Number(Number::from(u64::try_from(attempted).unwrap_or(u64::MAX)));
    match outcome {
        CallOutcome::Response(json) => CallOutcome::Response(with_header(json, value)),
        CallOutcome::ChatStream(mut stream) => {
            stream.model_response = with_header(stream.model_response, value);
            CallOutcome::ChatStream(stream)
        }
        CallOutcome::ResponsesStream(json) => {
            CallOutcome::ResponsesStream(with_header(json, value))
        }
    }
}
fn with_header(json: Json, value: Json) -> Json {
    match json {
        Json::Object(mut fields) => {
            let hidden = fields
                .entry(String::from(HIDDEN_PARAMS))
                .or_insert_with(|| Json::Object(IndexMap::new()));
            match hidden {
                Json::Object(hidden_fields) => {
                    let headers = hidden_fields
                        .entry(String::from(ADDITIONAL_HEADERS))
                        .or_insert_with(|| Json::Object(IndexMap::new()));
                    match headers {
                        Json::Object(header_fields) => {
                            header_fields.insert(String::from(ATTEMPTED_FALLBACKS_HEADER), value);
                        }
                        Json::Null
                        | Json::Bool(_)
                        | Json::Number(_)
                        | Json::Text(_)
                        | Json::List(_) => {}
                    }
                }
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {}
            }
            Json::Object(fields)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => json,
    }
}
