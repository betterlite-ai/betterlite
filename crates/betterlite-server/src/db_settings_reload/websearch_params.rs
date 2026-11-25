use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, validated_max_agentic_loops,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::websearch_request_hooks::Settings as WebsearchParams;
use betterlite_openai::openai_base_config::Object as JsonMap;
use thiserror::Error;

const LOOP_FIELD: &str = "websearch_interception_params.max_agentic_loops";

#[derive(Debug, Error)]
pub enum WebsearchParamError {
    #[error("websearch_interception_params.enabled_providers must be a list of provider names")]
    EnabledProvider,
    #[error("websearch_interception_params.search_tool_name must be a string")]
    SearchToolName,
    #[error(transparent)]
    MaxAgenticLoops(#[from] AgenticLoopCeilingError),
}

// _websearch_handler_params: "enabled" gates the callback and an enabled_providers that is not a
// non-empty list is dropped, so the handler's own defaults apply.
pub(crate) fn handler_params(stored: &JsonMap) -> Result<WebsearchParams, WebsearchParamError> {
    let defaults = WebsearchParams::default();
    let enabled_providers = match stored.get("enabled_providers") {
        Some(JsonValue::Array(items)) if !items.is_empty() => items
            .iter()
            .map(provider_name)
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) | None => defaults.enabled_providers,
    };
    let search_tool_name = match stored.get("search_tool_name") {
        Some(JsonValue::String(name)) => Some(name.clone()),
        Some(JsonValue::Null) | None => None,
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => {
            return Err(WebsearchParamError::SearchToolName);
        }
    };
    let ceiling = stored
        .get("max_agentic_loops")
        .map_or(AgenticLoopValue::Null, loop_value);
    let max_agentic_loops = validated_max_agentic_loops(&ceiling, LOOP_FIELD)?;
    Ok(WebsearchParams {
        enabled_providers,
        search_tool_name,
        max_agentic_loops,
    })
}

fn provider_name(value: &JsonValue) -> Result<String, WebsearchParamError> {
    match value {
        JsonValue::String(name) => Ok(name.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Err(WebsearchParamError::EnabledProvider),
    }
}

// A list or object is no whole number either; it rejects through the same NotWholeNumber error.
fn loop_value(value: &JsonValue) -> AgenticLoopValue {
    match value {
        JsonValue::Null => AgenticLoopValue::Null,
        JsonValue::Bool(flag) => AgenticLoopValue::Boolean(*flag),
        JsonValue::Number(number) => number.as_i64().map_or_else(
            || {
                number
                    .as_f64()
                    .map_or(AgenticLoopValue::Null, AgenticLoopValue::Float)
            },
            AgenticLoopValue::Integer,
        ),
        JsonValue::String(text) => AgenticLoopValue::Text(text.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => AgenticLoopValue::Text(value.to_string()),
    }
}
