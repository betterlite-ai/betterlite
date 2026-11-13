use crate::helpers::HelperResult;

use betterlite_anthropic::count_tokens::wire::transform_request;
use input::{Function, Input, Parts};

mod count;
mod fixture;
mod input;
mod tests;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(input::failure)?;
        let cases = &input.cases;
        match input.function {
            Function::Validate => count::answers("results", cases, count::validate_case),
            Function::Transform => count::answers("bodies", cases, |case| {
                let parts = Parts::of(case)?;
                serde_json::to_value(transform_request(&parts.body(&case.model)))
                    .map_err(input::failure)
            }),
            Function::Headers => count::answers("headers", cases, count::headers_case),
            Function::Handle => {
                count::answers("calls", cases, |case| count::handle_case(case, &runtime))
            }
            Function::Counter => {
                count::answers("calls", cases, |case| count::counter_case(case, &runtime))
            }
            Function::Factory => {
                count::answers("cases", cases, |case| count::factory_case(case, &runtime))
            }
        }
    }
}
