use crate::arguments::json::Json;

// Anthropic's client tool format has no required "type", so only an OpenAI-shaped function tool matches on function.name.
#[must_use]
pub fn has_tool_with_name(tools: &[Json], tool_name: &str) -> bool {
    for tool in tools {
        if tool_matches(tool, tool_name) {
            return true;
        }
    }
    false
}

fn tool_matches(tool: &Json, tool_name: &str) -> bool {
    let Json::Object(fields) = tool else {
        return false;
    };
    if fields.get("type").and_then(Json::as_str) == Some("function")
        && let Some(function) = fields.get("function").and_then(Json::as_object)
    {
        return function.get("name").and_then(Json::as_str) == Some(tool_name);
    }
    fields.get("name").and_then(Json::as_str) == Some(tool_name)
}

mod tests;
