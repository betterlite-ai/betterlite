use crate::helpers::HelperResult;

use input::Input;

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
        match input {
            Input::Validate { cases } => count::validate(cases),
            Input::Transform { cases } => count::transform(cases),
            Input::Headers { cases } => count::headers(cases),
            Input::Handle { cases } => count::handle(cases, &runtime),
            Input::Counter { cases } => count::counter(cases, &runtime),
            Input::Factory { cases } => count::factory(cases, &runtime),
        }
    }
}
