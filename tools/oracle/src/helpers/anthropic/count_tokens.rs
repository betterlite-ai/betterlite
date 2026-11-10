use crate::helpers::HelperResult;

use input::{Function, Input};

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
        match input.function {
            Function::Validate => count::validate(&input.cases),
            Function::Transform => count::transform(&input.cases),
            Function::Headers => count::headers(&input.cases),
            Function::Handle => count::handle(&input.cases, &runtime),
            Function::Counter => count::counter(&input.cases, &runtime),
            Function::Factory => count::factory(&input.cases, &runtime),
        }
    }
}
