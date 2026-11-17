use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::pass_through_logging::tinyfish::sse::sse_poller_spawned;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::ports::RequestLog;

// What the sink was handed: the poller dispatches, while the blocking and fallback handlers return their payload.
#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub cache_hit: Option<bool>,
    pub response: Option<String>,
}

// LiteLLM writes the billed model, provider and cost onto the logging object, and the oracle compares those details.
pub(super) fn billing_view(
    logging_obj: &StreamLoggingObject,
    logged: Recorded,
    requests: RequestLog,
) -> Result<Value, Box<HelperError>> {
    let Some((details, call_id)) = logging_obj.read(&mut |state| {
        (
            state.model_call_details.clone(),
            state.litellm_call_id.clone(),
        )
    }) else {
        return Err(super::failure("the logging object holds no state"));
    };
    Ok(json!({
        "model": text_entry(&details, "model").map(Value::String),
        "custom_llm_provider": text_entry(&details, "custom_llm_provider").map(Value::String),
        "response_cost": text_entry(&details, "response_cost").map(Value::String),
        "litellm_call_id": call_id,
        "cache_hit": logged.cache_hit,
        "response": parsed_response(logged.response.as_deref())?,
        "sse_poller_spawned": sse_poller_spawned(logging_obj),
        "requests": {
            "count": requests.count,
            "paths": requests.paths,
            "api_keys": requests.api_keys,
        },
    }))
}

// The blocking and fallback handlers hand their payload back instead of dispatching, so the payload is read directly.
pub(super) fn outcome_logged(outcome: &HandlerOutcome) -> Recorded {
    Recorded {
        cache_hit: None,
        response: outcome_text(outcome),
    }
}

// LiteLLM serialises the parsed run with json.dumps and everything else is already text, so both sides parse it back.
fn parsed_response(text: Option<&str>) -> Result<Value, Box<HelperError>> {
    let Some(found) = text.filter(|found| !found.is_empty()) else {
        return Ok(Value::Null);
    };
    match serde_json::from_str::<Json>(found) {
        Ok(parsed) => serde_json::to_value(parsed).or_value_error(),
        Err(_error) => Ok(Value::String(found.to_owned())),
    }
}

fn outcome_text(outcome: &HandlerOutcome) -> Option<String> {
    let PassThroughResponse::Standard(response) = outcome.result.as_ref()? else {
        return None;
    };
    match &response.response {
        Json::Object(entries) => text_entry(entries, "response"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// LiteLLM writes the model and provider as text and the cost as a number, and the view compares all three as text.
pub(super) fn text_entry(fields: &IndexMap<String, Json>, name: &str) -> Option<String> {
    let Some(value) = fields.get(name) else {
        return None;
    };
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}
