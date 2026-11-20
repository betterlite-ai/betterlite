mod requests;
mod structured_answers;
mod targets;
mod write_back_answers;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::anthropic_translation::structured_write_back::defer_tool_exchange_systems;
use serde_json::{Value, json};

use requests::{Function, Request, Scenario};
use structured_answers::{
    fold_answer, hoisted_answer, structured_answer, system_conversion_answer,
};
use write_back_answers::{
    message_text_answer, responses_answer, system_block_answer, tool_calls_answer, tool_use_answer,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Request;

    fn run(input: &Self::Input) -> HelperResult {
        match &input.function {
            Function::SystemBlock => answers(&input.scenarios, system_block_answer),
            Function::MessageText => answers(&input.scenarios, message_text_answer),
            Function::ResponsesToInput => answers(&input.scenarios, responses_answer),
            Function::ToolUse => answers(&input.scenarios, tool_use_answer),
            Function::ToolCalls => answers(&input.scenarios, tool_calls_answer),
            Function::Hoisted => answers(&input.scenarios, hoisted_answer),
            Function::SystemConversion => answers(&input.scenarios, system_conversion_answer),
            Function::Fold => answers(&input.scenarios, fold_answer),
            Function::Defer => {
                let mut output = Vec::with_capacity(input.scenarios.len());
                for scenario in &input.scenarios {
                    output.push(json!({
                        "messages": defer_tool_exchange_systems(&scenario.messages)
                    }));
                }
                Ok(Value::Array(output))
            }
            Function::Structured => answers(&input.scenarios, structured_answer),
        }
    }
}

fn answers(
    scenarios: &[Scenario],
    answer: fn(&Scenario) -> Result<Value, Box<HelperError>>,
) -> HelperResult {
    let mut output = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        output.push(answer(scenario)?);
    }
    Ok(Value::Array(output))
}
