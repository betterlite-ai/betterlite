use betterlite_prompts::arguments::json::{Json, Object};

use super::ToolsError;
use super::fields::{function_fields, non_null, string_list};
use super::wire::{CUSTOM_TYPE, Options};

const SEARCH_TYPES: [&str; 2] = [
    "tool_search_tool_regex_20251119",
    "tool_search_tool_bm25_20251119",
];
const NO_DEFER_OR_CALLERS_TYPES: [&str; 4] = [
    "tool_search_tool_regex_20251119",
    "tool_search_tool_bm25_20251119",
    "computer_20241022",
    "computer_20250124",
];

// the four optional blocks run on the mapped tool, so a hosted tool shadowing a search type is suppressed too
pub(super) fn collect_options(tool: &Object, output_kind: &str) -> Result<Options, ToolsError> {
    let nested = function_fields(tool)?;
    let is_search = SEARCH_TYPES.contains(&output_kind);
    let suppressed = NO_DEFER_OR_CALLERS_TYPES.contains(&output_kind);
    let (defer_loading, allowed_callers) = if suppressed {
        (None, None)
    } else {
        (defer(tool, nested)?, callers(tool, nested)?)
    };
    Ok(Options {
        cache_control: if is_search { None } else { cache(tool, nested) },
        defer_loading,
        allowed_callers,
        input_examples: if output_kind == CUSTOM_TYPE {
            examples(tool, nested)
        } else {
            None
        },
    })
}

fn cache(tool: &Object, nested: Option<&Object>) -> Option<Json> {
    if let Some(present) = non_null(tool.get("cache_control")) {
        return Some(present.clone());
    }
    match nested.and_then(|function| function.get("cache_control")) {
        Some(value @ Json::Object(_)) => Some(value.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    }
}

// a top-level value wins whenever it is present, so only a missing or null one falls to the function
fn top_or_nested<'fields>(
    tool: &'fields Object,
    nested: Option<&'fields Object>,
    key: &str,
) -> Option<&'fields Json> {
    if let Some(present) = non_null(tool.get(key)) {
        return Some(present);
    }
    let function = nested?;
    non_null(function.get(key))
}

fn defer(tool: &Object, nested: Option<&Object>) -> Result<Option<bool>, ToolsError> {
    match top_or_nested(tool, nested, "defer_loading") {
        None => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(ToolsError::DeferLoading),
    }
}

fn callers(tool: &Object, nested: Option<&Object>) -> Result<Option<Vec<String>>, ToolsError> {
    top_or_nested(tool, nested, "allowed_callers")
        .map(|value| string_list(value, "allowed_callers"))
        .transpose()
}

// a wrong-shaped list is ignored here, so a top-level object still falls back to the nested list
fn examples(tool: &Object, nested: Option<&Object>) -> Option<Vec<Json>> {
    let top = non_null(tool.get("input_examples")).and_then(as_list);
    let inner = nested
        .and_then(|function| non_null(function.get("input_examples")))
        .and_then(as_list);
    top.or(inner)
}

fn as_list(value: &Json) -> Option<Vec<Json>> {
    match value {
        Json::List(items) => Some(items.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}
