use std::collections::BTreeMap;

use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_mcp_input::{create_follow_up_input, is_persistence_disabled};
use betterlite_core::api::responses_mcp_results::McpToolResult;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    Store {
        call_params: BTreeMap<String, Value>,
    },
    FollowUp {
        response: Box<ResponsesApiResponse>,
        tool_results: Vec<McpToolResult>,
        original_input: Option<ResponsesInput>,
        preserve_reasoning: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(answer)
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(Value::Array(answers))
    }
}

fn answer(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Store { call_params } => {
            let decoded = store_call(call_params)?;
            Ok(json!(is_persistence_disabled(&decoded)))
        }
        Call::FollowUp {
            response,
            tool_results,
            original_input,
            preserve_reasoning,
        } => {
            let items = create_follow_up_input(
                response,
                tool_results,
                original_input.as_ref(),
                *preserve_reasoning,
            )
            .or_value_error()?;
            Ok(serde_json::to_value(items)?)
        }
    }
}

// the store check reads a decoded request, so the case's call parameters are completed with the two required keys
fn store_call(params: &BTreeMap<String, Value>) -> Result<ResponsesCreateCall, Box<HelperError>> {
    let mut fields: Map<String, Value> = params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    fields.insert(String::from("model"), Value::String(String::from("oracle")));
    fields.insert(String::from("input"), Value::String(String::new()));
    serde_json::from_value(Value::Object(fields)).or_value_error()
}
