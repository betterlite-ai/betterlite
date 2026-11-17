use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::response_content::wire::{AnthropicToolCall, json_text};
use crate::tool_names::REVERSE_MAP_KEY;

use super::error::ResponseError;

// LiteLLM reads the reverse map only when the stored value is a dict, so any other value means no rewrites
pub(super) fn reverse_map_of<CounterFailure>(
    params: &IndexMap<String, Json>,
) -> Result<Option<IndexMap<String, String>>, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let fields = match params.get(REVERSE_MAP_KEY) {
        Some(Json::Object(fields)) => fields,
        None
        | Some(Json::Null | Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Ok(None);
        }
    };
    let mut reverse = IndexMap::with_capacity(fields.len());
    for (sanitized, original) in fields {
        let Some(target) = json_text(original) else {
            return Err(ResponseError::Parameter {
                field: REVERSE_MAP_KEY,
            });
        };
        reverse.insert(sanitized.clone(), target.to_owned());
    }
    Ok(Some(reverse))
}

pub(super) fn apply_reverse_map(
    tool_calls: &mut [AnthropicToolCall],
    reverse: &IndexMap<String, String>,
) {
    for call in tool_calls {
        let Some(name) = call.function.name.as_deref() else {
            continue;
        };
        if let Some(original) = reverse.get(name) {
            call.function.name = Some(original.clone());
        }
    }
}

// LiteLLM passes optional_params["speed"] straight to usage, whose declared shape is a string or nothing
pub(super) fn speed_of<CounterFailure>(
    params: &IndexMap<String, Json>,
) -> Result<Option<&str>, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    match params.get("speed") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(speed)) => Ok(Some(speed.as_str())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ResponseError::Parameter { field: "speed" })
        }
    }
}
