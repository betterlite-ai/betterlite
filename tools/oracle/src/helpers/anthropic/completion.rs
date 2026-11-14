use crate::helpers::{Helper, HelperResult};
use input::Input;

mod answers;
mod fixture;
mod input;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Select { models } => Ok(answers::select(models)),
            Input::Prompt { cases } => answers::prompts(cases),
            Input::Auth { cases } => answers::auth(cases),
            Input::Options { constructor, cases } => answers::options(constructor, cases),
            Input::Reply { cases } => answers::replies(cases),
            Input::Chunk { chunks } => answers::chunks(chunks),
            Input::Error { cases } => Ok(answers::errors(cases)),
        }
    }
}
