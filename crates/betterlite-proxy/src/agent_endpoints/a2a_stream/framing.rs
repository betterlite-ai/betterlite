use betterlite_core::agents::json::AgentJson;

use super::ports::StreamEventConverter;

pub const INVALID_PARAMS: i64 = -32_602;
pub const INTERNAL_ERROR: i64 = -32_603;

// a guardrail that ends a stream hands over the JSON-RPC object as text
#[must_use]
pub fn to_jsonrpc_object(chunk: AgentJson) -> AgentJson {
    match chunk {
        AgentJson::String(text) => {
            serde_json::from_str(&text).unwrap_or_else(|_unparsable| AgentJson::String(text))
        }
        other @ (AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_)) => other,
    }
}

#[must_use]
pub fn error_event(request_id: &AgentJson, code: i64, message: &str) -> AgentJson {
    let error = [
        (String::from("code"), AgentJson::from(code)),
        (String::from("message"), AgentJson::from(message)),
    ];
    AgentJson::Object(
        [
            (String::from("jsonrpc"), AgentJson::from("2.0")),
            (String::from("id"), request_id.clone()),
            (
                String::from("error"),
                AgentJson::Object(error.into_iter().collect()),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

// only a JSON object is an event the converter understands; any other value passes as it is
#[must_use]
pub fn convert_event<C>(
    converter: &C,
    event: AgentJson,
    request_id: Option<&AgentJson>,
) -> AgentJson
where
    C: StreamEventConverter,
{
    match event {
        AgentJson::Object(object) => AgentJson::Object(
            converter
                .convert(object.into_iter().collect(), request_id)
                .into_iter()
                .collect(),
        ),
        other @ (AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::String(_)
        | AgentJson::Array(_)) => other,
    }
}

mod tests;
