use betterlite_models::decisions::Json;

use super::StreamEventConverter;
use super::chunk::Branch;
use super::error::ShapeError;
use crate::response_content::decode_json_mode_arguments;
use crate::response_content::wire::format_json_argument;
use crate::usage::ReasoningTokenCounter;

const RESPONSE_FORMAT_TOOL_NAME: &str = "json_tool_call";

pub(crate) fn apply<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    branch: &mut Branch,
) -> Result<(), ShapeError>
where
    Counter: ReasoningTokenCounter,
{
    let Some(call) = branch.tool_call.as_ref() else {
        return Ok(());
    };
    if converter.state.json_mode != Some(true) {
        return Ok(());
    }
    if call.id.is_some() {
        converter.state.is_response_format_tool =
            call.function.name.as_deref() == Some(RESPONSE_FORMAT_TOOL_NAME);
    }
    if !converter.state.is_response_format_tool {
        return Ok(());
    }
    branch.content = conversion_content(&call.function.arguments)?;
    branch.tool_call = None;
    converter.state.converted_response_format_tool = true;
    Ok(())
}

// The streamed arguments arrive a fragment at a time, so each one is parsed on its own.
fn conversion_content(arguments: &str) -> Result<String, ShapeError> {
    let Ok(parsed) = decode_json_mode_arguments(arguments) else {
        return Ok(arguments.to_owned());
    };
    if let Json::Object(fields) = &parsed
        && let Some(values) = fields
            .get("values")
            .filter(|value| !matches!(value, Json::Null))
    {
        return format_json_argument(values).map_err(|source| ShapeError::Encode { source });
    }
    format_json_argument(&parsed).map_err(|source| ShapeError::Encode { source })
}
