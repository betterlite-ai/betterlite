use betterlite_models::decisions::Json;
use serde::Serialize;

use crate::spend_event::event::Mapping;
use crate::spend_event::input::SuccessKwargs;
use crate::spend_event::response::ToolKind;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FunctionName {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    pub kind: ToolKind,
    pub function: FunctionName,
}

// Chat tools nest the name under function, and Anthropic and Responses tools keep it at the top
fn tool_name(tool: &Mapping) -> Option<String> {
    let name = match tool.get("function") {
        Some(Json::Object(function)) => function.get("name"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => tool.get("name"),
    };
    let Some(Json::Text(text)) = name else {
        return None;
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

// Ordinary tools come before passthrough tools, and a name used twice stays twice
#[must_use]
pub fn request_tool_names(kwargs: &SuccessKwargs) -> Vec<String> {
    let passthrough = kwargs
        .passthrough_logging_payload
        .as_ref()
        .and_then(|payload| payload.request_body.as_ref())
        .and_then(|body| body.tools.as_ref());
    kwargs
        .tools
        .iter()
        .flatten()
        .chain(passthrough.into_iter().flatten())
        .filter_map(tool_name)
        .collect()
}

#[must_use]
pub fn function_tools(names: Vec<String>) -> Vec<FunctionTool> {
    names
        .into_iter()
        .map(|name| FunctionTool {
            kind: ToolKind::Function,
            function: FunctionName { name },
        })
        .collect()
}
