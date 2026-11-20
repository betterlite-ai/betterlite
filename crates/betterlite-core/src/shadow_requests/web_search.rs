use betterlite_models::decisions::Json;

use super::Object;

mod tests;

const TOOLS: &str = "tools";
const TOOL_TYPE: &str = "type";
const WEB_SEARCH: &str = "web_search";
const WEB_SEARCH_OPTIONS: &str = "web_search_options";
const WEB_SEARCH_PREFIX: &str = "web_search_";

// B4111's is_web_search_tool_responses lives in betterlite-websearch, which depends on this crate, so its rules are mirrored here
#[must_use]
pub fn request_has_hosted_web_search(request: &Object) -> bool {
    if request
        .get(WEB_SEARCH_OPTIONS)
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return true;
    }
    let Some(Json::List(tools)) = request.get(TOOLS) else {
        return false;
    };
    tools.iter().any(|tool| match tool {
        Json::Object(fields) => hosted_tool(fields.get(TOOL_TYPE)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    })
}

fn hosted_tool(tool_type: Option<&Json>) -> bool {
    match tool_type {
        Some(Json::Text(name)) => {
            name.as_str() == WEB_SEARCH || name.starts_with(WEB_SEARCH_PREFIX)
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => false,
    }
}
