use betterlite_anthropic::completion::model;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use fixture::Fixture;
use input::Input;
use serde_json::json;

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
            Input::Select { models } => Ok(json!({
                "selected": models
                    .iter()
                    .map(String::as_str)
                    .map(model::selects)
                    .collect::<Vec<bool>>(),
            })),
            Input::Prompt { cases } => answers::prompts(&fixture, cases),
            Input::Auth { cases } => answers::auth(cases),
            Input::Options { constructor, cases } => answers::options(&fixture, constructor, cases),
            Input::Reply { cases } => answers::replies(&fixture, cases),
            Input::Chunk { chunks } => answers::chunks(chunks),
            Input::Error { cases } => Ok(answers::errors(cases)),
        }
    }
}
