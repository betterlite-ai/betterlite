use betterlite_context::non_default_params::schema_cleanup::{
    remove_additional_properties, remove_strict_from_schema,
};
use betterlite_models::controls::json_value::JsonValue;
use serde_json::Value;

use super::error::WatsonxChatError;
use crate::openai_base_config::Object;

pub const SUPPORTED_PARAMS: [&str; 15] = [
    "temperature",
    "max_tokens",
    "top_p",
    "frequency_penalty",
    "stop",
    "seed",
    "stream",
    "tools",
    "tool_choice",
    "logprobs",
    "top_logprobs",
    "n",
    "presence_penalty",
    "response_format",
    "reasoning_effort",
];

const TOOL_CHOICE_OPTIONS: [&str; 3] = ["auto", "none", "required"];

fn clean_tools(tools: &Value) -> Result<Value, WatsonxChatError> {
    let mut schema =
        serde_json::from_value::<JsonValue>(tools.clone()).map_err(WatsonxChatError::Json)?;
    remove_additional_properties(&mut schema);
    remove_strict_from_schema(&mut schema);
    serde_json::to_value(schema).map_err(WatsonxChatError::Json)
}

// LiteLLM pops tools and tool_choice before the OpenAI mapping, so the loop below never sees them
pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &mut Object,
) -> Result<(), WatsonxChatError> {
    match non_default_params.get("tool_choice") {
        None | Some(Value::Null) => {}
        Some(Value::String(choice)) if TOOL_CHOICE_OPTIONS.contains(&choice.as_str()) => {
            optional_params.insert(
                String::from("tool_choice_option"),
                Value::String(choice.clone()),
            );
        }
        Some(choice) => {
            optional_params.insert(String::from("tool_choice"), choice.clone());
        }
    }
    for (name, value) in non_default_params {
        if name != "tools" && name != "tool_choice" && SUPPORTED_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    match non_default_params.get("tools") {
        None | Some(Value::Null) => {}
        Some(tools) => {
            optional_params.insert(String::from("tools"), clean_tools(tools)?);
        }
    }
    Ok(())
}
