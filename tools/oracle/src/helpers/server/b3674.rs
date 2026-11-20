mod requests;
mod structured_answers;
mod targets;
mod write_back_answers;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::Value;

use requests::Request;
use structured_answers::{
    defer_answer, fold_answer, hoisted_answer, structured_answer, system_conversion_answer,
};
use write_back_answers::{
    message_text_answer, responses_answer, system_block_answer, tool_calls_answer, tool_use_answer,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Request;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Request::SystemBlock { scenarios } => answers(scenarios, system_block_answer),
            Request::MessageText { scenarios } => answers(scenarios, message_text_answer),
            Request::ResponsesToInput { scenarios } => answers(scenarios, responses_answer),
            Request::ToolUse { scenarios } => answers(scenarios, tool_use_answer),
            Request::ToolCalls { scenarios } => answers(scenarios, tool_calls_answer),
            Request::Hoisted { scenarios } => answers(scenarios, hoisted_answer),
            Request::SystemConversion { scenarios } => answers(scenarios, system_conversion_answer),
            Request::Fold { scenarios } => answers(scenarios, fold_answer),
            Request::Defer { scenarios } => answers(scenarios, defer_answer),
            Request::Structured { scenarios } => answers(scenarios, structured_answer),
        }
    }
}

fn answers<T>(scenarios: &[T], answer: fn(&T) -> Result<Value, Box<HelperError>>) -> HelperResult {
    let mut output = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        output.push(answer(scenario)?);
    }
    Ok(Value::Array(output))
}
