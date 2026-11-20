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
        let scenarios: &[Scenario] = &input.scenarios;
        match &input.function {
            Function::SystemBlock => answers(scenarios, |case| Ok(system_block_answer(case))),
            Function::MessageText => answers(scenarios, message_text_answer),
            Function::ResponsesToInput => answers(scenarios, responses_answer),
            Function::ToolUse => answers(scenarios, |case| Ok(tool_use_answer(case))),
            Function::ToolCalls => answers(scenarios, |case| Ok(tool_calls_answer(case))),
            Function::Hoisted => answers(scenarios, hoisted_answer),
            Function::SystemConversion => {
                answers(scenarios, |case| Ok(system_conversion_answer(case)))
            }
            Function::Fold => answers(scenarios, |case| Ok(fold_answer(case))),
            Function::Defer => {
                let mut output = Vec::with_capacity(scenarios.len());
                for scenario in scenarios {
                    output.push(json!({
                        "messages": defer_tool_exchange_systems(&scenario.messages)
                    }));
                }
                Ok(Value::Array(output))
            }
            Function::Structured => answers(scenarios, structured_answer),
        }
    }
}

fn answers<F>(scenarios: &[Scenario], answer: F) -> HelperResult
where
    F: Fn(&Scenario) -> Result<Value, Box<HelperError>>,
{
    let mut output = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        output.push(answer(scenario)?);
    }
    Ok(Value::Array(output))
}
