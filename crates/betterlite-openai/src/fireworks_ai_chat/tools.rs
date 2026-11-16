use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::definitions::legacy::{DEFS_MAX_INLINED_BYTES, unpack_legacy_defs};

use crate::fireworks_ai_chat::error::FireworksChatError;

pub(crate) const FUNCTION: &str = "function";
const TYPE: &str = "type";
const NAME: &str = "name";
const STRICT: &str = "strict";
const PARAMETERS: &str = "parameters";

// _transform_tools: only a function tool changes, losing strict and unpacking legacy definitions
pub(crate) fn transform_tools(tools: &mut Json) -> Result<(), FireworksChatError> {
    let Json::List(entries) = tools else {
        return Err(FireworksChatError::Tools);
    };
    for (index, entry) in entries.iter_mut().enumerate() {
        let Json::Object(fields) = entry else {
            return Err(FireworksChatError::Tool { index });
        };
        if fields.get(TYPE).and_then(Json::as_str) != Some(FUNCTION) {
            continue;
        }
        let Some(Json::Object(function)) = fields.get_mut(FUNCTION) else {
            return Err(FireworksChatError::Tool { index });
        };
        function.shift_remove(STRICT);
        if let Some(Json::Object(parameters)) = function.get_mut(PARAMETERS) {
            *parameters = unpack_legacy_defs(parameters.clone(), DEFS_MAX_INLINED_BYTES)?;
        }
    }
    Ok(())
}

// the declared names; LiteLLM's comprehension raises on a tool without one, so the repair is skipped
pub(crate) fn function_names(tools: &Json) -> Option<Vec<String>> {
    let Json::List(entries) = tools else {
        return None;
    };
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let function = entry.as_object()?.get(FUNCTION)?.as_object()?;
        if let Some(name) = function.get(NAME).and_then(Json::as_str) {
            names.push(name.to_owned());
        }
    }
    Some(names)
}
