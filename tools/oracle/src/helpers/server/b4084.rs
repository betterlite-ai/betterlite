use betterlite_server::shadow_calls::judge_prompt::{judge_user_prompt, tool_definitions_text};
use betterlite_server::shadow_calls::response_format::pairwise_judge_response_format;
use betterlite_server::shadow_calls::unmask::unmask_preference;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    conversation: String,
    #[serde(default)]
    response_a: String,
    #[serde(default)]
    response_b: String,
    #[serde(default)]
    tool_definitions: String,
    #[serde(default)]
    tools: Option<Value>,
    #[serde(default)]
    raw_preference: String,
    #[serde(default)]
    real_is_a: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    ToolDefinitions,
    JudgePrompt,
    Unmask,
    ResponseFormat,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let no_tools = Value::Null;
        let raw_tools = input.tools.as_ref().unwrap_or(&no_tools);
        match input.operation {
            Operation::ToolDefinitions => {
                let tools = serde_json::from_value(raw_tools.clone()).or_value_error()?;
                Ok(json!({"text": tool_definitions_text(&tools)}))
            }
            Operation::JudgePrompt => Ok(json!({
                "text": judge_user_prompt(
                    &input.conversation,
                    &input.response_a,
                    &input.response_b,
                    &input.tool_definitions,
                ),
            })),
            Operation::Unmask => Ok(json!({
                "preference": unmask_preference(&input.raw_preference, input.real_is_a),
            })),
            Operation::ResponseFormat => {
                Ok(json!({"response_format": pairwise_judge_response_format()}))
            }
        }
    }
}
