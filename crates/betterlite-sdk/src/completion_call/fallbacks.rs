use super::{error::CompletionCallError, request::args_to_map};
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::ports::IdSource;
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;
use crate::completion_request::args::CompletionArgs;
use crate::completion_request::call::FallbackCall;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
pub const ATTEMPTED_FALLBACKS_HEADER: &str = "x-litellm-attempted-fallbacks";
const HIDDEN_PARAMS: &str = "_hidden_params";
const ADDITIONAL_HEADERS: &str = "additional_headers";
const INTERNAL_PARAMS: [&str; 3] = [
    "skip_mcp_handler",
    "mcp_handler_context",
    "_skip_mcp_handler",
];
pub async fn run_fallbacks(
    requester: &dyn CallTarget,
    call: FallbackCall,
    logging: Option<&LoggingHandle>,
    ids: &dyn IdSource,
) -> Result<CallOutcome, CompletionCallError> {
    let original = call.args.model.clone().unwrap_or_default();
    let base = fallback_base(&call.args, ids)?;
    let mut candidates = vec![Json::Text(original.clone())];
    candidates.extend(call.fallbacks);
    let mut last_error: Option<String> = None;
    for (position, candidate) in candidates.iter().enumerate() {
        let (model, chosen) = match candidate {
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
        let mut attempt = filter_internal_params(chosen);
        attempt.insert(String::from("model"), Json::Text(model.clone()));
        match requester.call(attempt, logging).await {
            Ok(outcome) if !matches!(&outcome, CallOutcome::Response(Json::Null)) => {
                return Ok(with_attempt_header(outcome, position));
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(model = %model, error = %error, "Fallback attempt failed");
                last_error = Some(error.to_string());
            }
        }
    }
    let last = last_error.unwrap_or_else(|| String::from("None"));
    Err(CompletionCallError::AllFallbacksFailed { last })
}
fn fallback_base(
    args: &CompletionArgs,
    ids: &dyn IdSource,
) -> Result<RequestMap, CompletionCallError> {
    let mut base = args_to_map(args)?;
    base.shift_remove("model");
    base.shift_remove("acompletion");
    base.shift_remove("fallbacks");
    let id = ids.next_call_id();
    base.insert(String::from("litellm_call_id"), Json::Text(id));
    Ok(filter_internal_params(base))
}
fn overlay(mut base: RequestMap, fields: &IndexMap<String, Json>) -> RequestMap {
    for (name, value) in fields {
        base.insert(name.clone(), value.clone());
    }
    base
}
fn model_name(value: Json) -> Result<String, CompletionCallError> {
    if let Json::Text(name) = value {
        return Ok(name);
    }
    Err(CompletionCallError::Field { field: "model" })
}
fn filter_internal_params(mut request: RequestMap) -> RequestMap {
    for name in INTERNAL_PARAMS {
        request.shift_remove(name);
    }
    request
}
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
    let Json::Object(mut fields) = json else {
        return json;
    };
    let hidden = fields
        .entry(String::from(HIDDEN_PARAMS))
        .or_insert_with(|| Json::Object(IndexMap::new()));
    if let Json::Object(hidden_fields) = hidden {
        let headers = hidden_fields
            .entry(String::from(ADDITIONAL_HEADERS))
            .or_insert_with(|| Json::Object(IndexMap::new()));
        if let Json::Object(header_fields) = headers {
            header_fields.insert(String::from(ATTEMPTED_FALLBACKS_HEADER), value);
        }
    }
    Json::Object(fields)
}
