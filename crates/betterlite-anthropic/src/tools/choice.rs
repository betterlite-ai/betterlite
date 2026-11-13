use betterlite_prompts::arguments::json::{Json, Object};

use super::ToolsError;
use super::wire::{ChoiceKind, ToolChoice};

pub(super) fn named_mode(mode: &str) -> Option<ToolChoice> {
    let kind = match mode {
        "auto" => ChoiceKind::Auto,
        "required" => ChoiceKind::Any,
        "none" => ChoiceKind::None,
        _ => return None,
    };
    Some(ToolChoice {
        kind,
        name: None,
        disable_parallel_tool_use: None,
    })
}

pub(super) fn object_choice(fields: &Object) -> Result<Option<ToolChoice>, ToolsError> {
    if fields.contains_key("type") && !fields.contains_key("function") {
        let kind = match fields.get("type").and_then(Json::as_str) {
            Some("auto") => Some(ChoiceKind::Auto),
            Some("required" | "any") => Some(ChoiceKind::Any),
            Some("none") => Some(ChoiceKind::None),
            Some(_) | None => None,
        };
        return Ok(kind.map(|selected| ToolChoice {
            kind: selected,
            name: None,
            disable_parallel_tool_use: None,
        }));
    }
    let function = match fields.get("function") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(function)) => function,
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "tool_choice.function",
                expected: "an object",
            });
        }
    };
    let name = match function.get("name") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(text)) => text,
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "tool_choice.function.name",
                expected: "a string",
            });
        }
    };
    Ok(Some(ToolChoice {
        kind: ChoiceKind::Tool,
        name: Some(name.clone()),
        disable_parallel_tool_use: None,
    }))
}
