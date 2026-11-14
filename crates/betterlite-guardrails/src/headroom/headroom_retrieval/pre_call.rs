use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::tools;

pub const CONVERTED_STREAM_KEY: &str = "_headroom_interception_converted_stream";
const STREAM_KEY: &str = "stream";
const BACKGROUND_KEY: &str = "background";
const TOOLS_KEY: &str = "tools";

// LiteLLM's _STREAM_CONVERTIBLE_CALL_TYPES: the loop only sees a whole response on these routes
const STREAM_CONVERTIBLE_CALL_TYPES: [CallType; 4] = [
    CallType::Completion,
    CallType::Acompletion,
    CallType::Responses,
    CallType::Aresponses,
];

// the base hook runs first, and its replacement is what the guards below inspect and return
#[must_use]
pub fn pre_call_deployment_hook(
    base_result: Option<&IndexMap<String, Json>>,
    kwargs: &IndexMap<String, Json>,
    call_type: Option<CallType>,
) -> Option<IndexMap<String, Json>> {
    let convertible = call_type.is_some_and(|kind| STREAM_CONVERTIBLE_CALL_TYPES.contains(&kind));
    if !convertible {
        return base_result.cloned();
    }
    let effective = base_result.unwrap_or(kwargs);
    let streams = matches!(effective.get(STREAM_KEY), Some(Json::Bool(true)));
    let background = matches!(effective.get(BACKGROUND_KEY), Some(Json::Bool(true)));
    if !streams || background {
        return base_result.cloned();
    }
    if !tools::value_holds_retrieve_tool(effective.get(TOOLS_KEY)) {
        return base_result.cloned();
    }
    let mut converted = effective.clone();
    converted.insert(STREAM_KEY.to_owned(), Json::Bool(false));
    converted.insert(CONVERTED_STREAM_KEY.to_owned(), Json::Bool(true));
    Some(converted)
}
