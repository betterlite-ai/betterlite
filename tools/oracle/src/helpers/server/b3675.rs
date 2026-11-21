mod answers;
mod requests;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::Value;

use answers::{
    extract_answer, image_sources_answer, output_apply_answer, post_call_hook_answer,
    structured_messages_answer, top_level_system_answer,
};
use requests::{Function, Request, Scenario};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Request;

    fn run(input: &Self::Input) -> HelperResult {
        let mut output = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            output.push(answer(&input.function, scenario)?);
        }
        Ok(Value::Array(output))
    }
}

fn answer(function: &Function, scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    match function {
        Function::Extract => Ok(extract_answer(scenario)),
        Function::ImageSources => Ok(image_sources_answer(scenario)),
        Function::TopLevelSystem => Ok(top_level_system_answer(scenario)),
        Function::StructuredMessages => structured_messages_answer(scenario),
        Function::OutputApply => output_apply_answer(scenario),
        Function::PostCallHook => post_call_hook_answer(scenario),
    }
}
