use crate::helpers::{Helper, HelperResult};
use fixture::Fixture;
use input::Input;

mod answers;
mod fixture;
mod input;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::new()?;
        match input {
            Input::Select { models } => Ok(answers::select(models)),
            Input::Prompt { cases } => answers::prompts(&fixture, cases),
            Input::Auth { cases } => answers::auth(cases),
            Input::Options { constructor, cases } => answers::options(&fixture, constructor, cases),
            Input::Reply { cases } => answers::replies(&fixture, cases),
            Input::Chunk { chunks } => answers::chunks(chunks),
            Input::Error { cases } => Ok(answers::errors(cases)),
        }
    }
}
