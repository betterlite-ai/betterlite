use betterlite_models::decisions::Json as ParamsJson;
use betterlite_prompts::arguments::json::{Json, Object};

use super::{FeatureFlagsError, FeatureParams};

pub const CODE_EXECUTION_TOOL: &str = "code_execution_20250825";

#[must_use]
pub fn is_computer_tool_used(tools: Option<&[Json]>) -> Option<String> {
    if let Some(present) = tools {
        return present.iter().find_map(computer_tool_version);
    }
    None
}

fn computer_tool_version(tool: &Json) -> Option<String> {
    let fields = tool.as_object()?;
    let kind = fields.get("type").and_then(Json::as_str)?;
    kind.starts_with("computer_").then(|| kind.to_owned())
}

#[must_use]
pub fn is_web_search_tool_used(tools: Option<&[Json]>) -> bool {
    any_tool(tools, |fields| {
        tool_kind(fields).is_some_and(|kind| kind.starts_with("web_search"))
    })
}

#[must_use]
pub fn is_tool_search_used(tools: Option<&[Json]>) -> bool {
    any_tool(tools, |fields| {
        matches!(
            tool_kind(fields),
            Some("tool_search_tool_regex_20251119" | "tool_search_tool_bm25_20251119")
        )
    })
}

#[must_use]
pub fn is_code_execution_tool_used(tools: Option<&[Json]>) -> bool {
    any_tool(tools, |fields| {
        tool_kind(fields) == Some(CODE_EXECUTION_TOOL)
    })
}

#[must_use]
pub fn uses_programmatic_calls(tools: Option<&[Json]>) -> bool {
    any_tool(tools, |fields| {
        allowed_calls_code_execution(fields.get("allowed_callers"))
            || function_allows_code_execution(fields)
    })
}

fn function_allows_code_execution(fields: &Object) -> bool {
    let Some(Json::Object(function)) = fields.get("function") else {
        return false;
    };
    allowed_calls_code_execution(function.get("allowed_callers"))
}

fn allowed_calls_code_execution(value: Option<&Json>) -> bool {
    let Some(Json::List(callers)) = value else {
        return false;
    };
    callers
        .iter()
        .any(|caller| matches!(caller, Json::Text(text) if text == CODE_EXECUTION_TOOL))
}

#[must_use]
pub fn uses_input_examples(tools: Option<&[Json]>) -> bool {
    any_tool(tools, |fields| {
        has_examples(fields.get("input_examples")) || function_has_examples(fields)
    })
}

fn function_has_examples(fields: &Object) -> bool {
    let Some(Json::Object(function)) = fields.get("function") else {
        return false;
    };
    has_examples(function.get("input_examples"))
}

const fn has_examples(value: Option<&Json>) -> bool {
    let Some(Json::List(examples)) = value else {
        return false;
    };
    !examples.is_empty()
}

fn any_tool<F>(tools: Option<&[Json]>, matches: F) -> bool
where
    F: Fn(&Object) -> bool,
{
    let Some(present) = tools else {
        return false;
    };
    present
        .iter()
        .any(|tool| tool.as_object().is_some_and(&matches))
}

#[must_use]
pub fn is_container_with_skills_used(params: Option<&FeatureParams>) -> bool {
    let Some(ParamsJson::Object(container)) = params.and_then(|source| source.container.as_ref())
    else {
        return false;
    };
    matches!(container.get("skills"), Some(ParamsJson::List(skills)) if !skills.is_empty())
}

pub fn eager_input_streaming_flag(tool: &Json) -> Result<Option<bool>, FeatureFlagsError> {
    let Some(fields) = tool.as_object() else {
        return Ok(None);
    };
    let top = eager_bool(fields.get("eager_input_streaming"))?;
    let nested = match fields.get("function") {
        None | Some(Json::Null) => None,
        Some(function) => {
            let Some(function_fields) = function.as_object() else {
                return Err(FeatureFlagsError::EagerInputStreaming);
            };
            eager_bool(function_fields.get("eager_input_streaming"))?
        }
    };
    Ok(top.or(nested))
}

const fn eager_bool(value: Option<&Json>) -> Result<Option<bool>, FeatureFlagsError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(FeatureFlagsError::EagerInputStreaming),
    }
}

pub fn is_eager_input_streaming_used(tools: Option<&[Json]>) -> Result<bool, FeatureFlagsError> {
    let Some(present) = tools else {
        return Ok(false);
    };
    for tool in present {
        if eager_input_streaming_flag(tool)? == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[must_use]
pub fn is_native_web_search_tool(tool: &Json) -> bool {
    let Some(fields) = tool.as_object() else {
        return false;
    };
    tool_kind(fields).is_some_and(|kind| kind.starts_with("web_search_"))
}

fn tool_kind(fields: &Object) -> Option<&str> {
    match fields.get("type") {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) | None => None,
    }
}
