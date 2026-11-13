use crate::arguments::concatenated::salvage_concatenated_arguments;
use crate::arguments::json::{Json, Object};
use crate::arguments::parse::{ParseArgumentsError, parse_tool_call_arguments};

const REDACTED_BY_LITELLM: &str = "redacted-by-litellm";

pub(super) fn parse(raw: Option<&Json>, tool_name: Option<&str>, context: &str) -> Vec<Object> {
    match raw {
        Some(Json::Object(object)) => vec![object.clone()],
        Some(Json::Text(text)) => parse_text(text, tool_name, context),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => {
            vec![Object::new()]
        }
    }
}

// A model can truncate the JSON string or emit several argument objects back to back, so one string yields every object LiteLLM keeps.
fn parse_text(text: &str, tool_name: Option<&str>, context: &str) -> Vec<Object> {
    let normalized = if text == REDACTED_BY_LITELLM {
        "{}"
    } else {
        text
    };
    match parse_tool_call_arguments(normalized, tool_name, Some(context)) {
        Ok(Json::Object(object)) => vec![object],
        Ok(_) => vec![Object::new()],
        Err(error) => recover_concatenated(normalized, tool_name, context, &error),
    }
}

fn recover_concatenated(
    text: &str,
    tool_name: Option<&str>,
    context: &str,
    error: &ParseArgumentsError,
) -> Vec<Object> {
    let recovered = salvage_concatenated_arguments(text);
    if recovered.is_empty() {
        tracing::warn!(
            tool_name = tool_name.unwrap_or("<unknown>"),
            context,
            %error,
            "failed to parse tool call arguments"
        );
        return vec![Object::new()];
    }
    tracing::warn!(
        tool_name = tool_name.unwrap_or("<unknown>"),
        context,
        recovered = recovered.len(),
        "recovered concatenated tool call arguments"
    );
    recovered
}
