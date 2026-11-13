use betterlite_prompts::arguments::json::{Json, Object};

use super::ToolsError;
use super::fields::{function_fields, non_null, string_list};
use super::wire::CUSTOM_TYPE;

pub(super) const SEARCH_TYPES: [&str; 2] = [
    "tool_search_tool_regex_20251119",
    "tool_search_tool_bm25_20251119",
];
pub(super) const NO_DEFER_OR_CALLERS_TYPES: [&str; 4] = [
    "tool_search_tool_regex_20251119",
    "tool_search_tool_bm25_20251119",
    "computer_20241022",
    "computer_20250124",
];

pub(super) struct Options {
    pub cache_control: Option<Json>,
    pub defer_loading: Option<bool>,
    pub allowed_callers: Option<Vec<String>>,
    pub input_examples: Option<Vec<Json>>,
}

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

fn defer(tool: &Object, nested: Option<&Object>) -> Result<Option<bool>, ToolsError> {
    let top = non_null(tool.get("defer_loading"));
    let inner = nested.and_then(|function| non_null(function.get("defer_loading")));
    match top.or(inner) {
        None => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(ToolsError::DeferLoading),
    }
}

fn callers(tool: &Object, nested: Option<&Object>) -> Result<Option<Vec<String>>, ToolsError> {
    let top = non_null(tool.get("allowed_callers"))
        .map(string_list)
        .transpose()?;
    let inner = nested
        .and_then(|function| non_null(function.get("allowed_callers")))
        .map(string_list)
        .transpose()?;
    Ok(top.or(inner))
}

fn examples(tool: &Object, nested: Option<&Object>) -> Option<Vec<Json>> {
    let top = tool.get("input_examples").and_then(as_list);
    let inner = nested
        .and_then(|function| function.get("input_examples"))
        .and_then(as_list);
    top.or(inner)
}

fn as_list(value: &Json) -> Option<Vec<Json>> {
    match value {
        Json::List(items) => Some(items.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

pub(super) fn insert_option(fields: &mut Object, key: &str, value: Option<Json>) {
    if let Some(present) = value {
        fields.insert(key.to_owned(), present);
    }
}
