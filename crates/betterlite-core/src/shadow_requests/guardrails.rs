use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_logging::guardrail_snapshot::request_guardrail_fingerprint;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::Object;
use super::surfaces::Surface;

mod tests;

const BODY: &str = "body";
const INPUT: &str = "input";
const INSTRUCTIONS: &str = "instructions";
const LITELLM_PARAMS: &str = "litellm_params";
const MESSAGES: &str = "messages";
const PROXY_SERVER_REQUEST: &str = "proxy_server_request";
const SYSTEM: &str = "system";

// a guardrail may have rewritten the request after the logged kwargs were taken, so only a snapshot whose guardrail history is still current replaces them
#[must_use]
pub fn post_guardrail_kwargs(
    kwargs: &Object,
    request_metadata: &Object,
    surface: Surface,
    snapshot: Option<&GuardrailRequestSnapshot>,
) -> Option<Object> {
    let snapshot = snapshot?;
    let fingerprint = request_guardrail_fingerprint(request_metadata)?;
    if snapshot.fingerprint != fingerprint {
        return None;
    }
    let messages_key = match surface {
        Surface::Responses => INPUT,
        Surface::Chat | Surface::AnthropicMessages => MESSAGES,
    };
    let body = &snapshot.body;
    let messages = body.get(messages_key).cloned().unwrap_or(Json::Null);
    let system = body.get(SYSTEM).cloned().unwrap_or(Json::Null);
    let instructions = body.get(INSTRUCTIONS).cloned().unwrap_or(Json::Null);
    let mut result = kwargs.clone();
    result.insert(MESSAGES.to_owned(), messages);
    result.insert(SYSTEM.to_owned(), system);
    result.insert(INSTRUCTIONS.to_owned(), instructions);
    let mut parameters = object_or_empty(result.get(LITELLM_PARAMS));
    let mut request = object_or_empty(parameters.get(PROXY_SERVER_REQUEST));
    request.insert(BODY.to_owned(), Json::Object(snapshot.body.clone()));
    parameters.insert(PROXY_SERVER_REQUEST.to_owned(), Json::Object(request));
    result.insert(LITELLM_PARAMS.to_owned(), Json::Object(parameters));
    Some(result)
}

fn object_or_empty(value: Option<&Json>) -> IndexMap<String, Json> {
    match value {
        Some(Json::Object(fields)) => fields.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            IndexMap::new()
        }
    }
}
