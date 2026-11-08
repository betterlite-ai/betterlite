use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::schema::drop_non_python_regex_patterns;
use betterlite_prompts::schema::flatten::{flatten_top_level, tool_with_sanitized_parameters};

use super::OPENAI;

pub(super) fn sanitized_tools(
    optional_params: &Object,
    provider: Option<&str>,
    hosted: bool,
) -> Option<Vec<Json>> {
    let Some(Json::List(tools)) = optional_params.get("tools") else {
        return None;
    };
    if provider != Some(OPENAI) {
        return None;
    }
    let sanitize: fn(&Object) -> Object = if hosted {
        |schema| flatten_top_level(&drop_non_python_regex_patterns(schema))
    } else {
        drop_non_python_regex_patterns
    };
    Some(
        tools
            .iter()
            .map(|tool| sanitize_tool(tool, sanitize))
            .collect(),
    )
}

fn sanitize_tool(tool: &Json, sanitize: fn(&Object) -> Object) -> Json {
    match tool {
        Json::Object(object) => {
            Json::Object(tool_with_sanitized_parameters(object.clone(), sanitize))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            tool.clone()
        }
    }
}
