use betterlite_anthropic::tools::{map_tool_choice, map_tools};
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    Tools {
        tools: Vec<Json>,
    },
    ToolChoice {
        #[serde(default)]
        tool_choice: Option<Json>,
        #[serde(default)]
        parallel_tool_calls: Option<bool>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer(call)?);
        }
        Ok(json!({"calls": answers}))
    }
}

fn answer(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Tools { tools } => tools_answer(tools),
        Call::ToolChoice {
            tool_choice,
            parallel_tool_calls,
        } => choice_answer(tool_choice.as_ref(), *parallel_tool_calls),
    }
}

fn tools_answer(tools: &[Json]) -> Result<Value, Box<HelperError>> {
    let mapping = match map_tools(tools) {
        Ok(mapping) => mapping,
        Err(error) => return Ok(json!({"ok": false, "status": error.status_code()})),
    };
    let mcp_servers = if mapping.mcp_servers.is_empty() {
        Value::Null
    } else {
        serde_json::to_value(&mapping.mcp_servers).map_err(Box::<HelperError>::from)?
    };
    Ok(json!({
        "ok": true,
        "tools": serde_json::to_value(&mapping.tools).map_err(Box::<HelperError>::from)?,
        "mcp_servers": mcp_servers,
    }))
}

fn choice_answer(
    tool_choice: Option<&Json>,
    parallel_tool_calls: Option<bool>,
) -> Result<Value, Box<HelperError>> {
    let chosen = match map_tool_choice(tool_choice, parallel_tool_calls) {
        Ok(choice) => choice,
        Err(error) => return Ok(json!({"ok": false, "status": error.status_code()})),
    };
    let mapped = chosen
        .map(|choice| serde_json::to_value(choice).map_err(Box::<HelperError>::from))
        .transpose()?;
    Ok(json!({"ok": true, "tool_choice": mapped}))
}
