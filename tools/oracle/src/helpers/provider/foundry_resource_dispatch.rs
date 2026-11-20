use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod image_edit;
mod image_generation;
mod messages_count;
mod passthrough;
mod rerank;
mod responses_decisions;
mod support;
mod tests;
mod vector_search;

pub(crate) struct Op;

// The oracle calls one operation per input; `case` names the family the way the Python adapter does
#[derive(Debug, Deserialize)]
#[serde(tag = "case", rename_all = "snake_case")]
pub(crate) enum Input {
    ImageGeneration(image_generation::Input),
    ImageEdit(image_edit::Input),
    Rerank(rerank::Input),
    ResponsesDecisions(responses_decisions::Input),
    VectorSearch {},
    Passthrough(passthrough::Input),
    MessagesCount(messages_count::Input),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ImageGeneration(values) => image_generation::run(values),
            Input::ImageEdit(values) => image_edit::run(values),
            Input::Rerank(values) => rerank::run(values),
            Input::ResponsesDecisions(values) => responses_decisions::answer(values),
            Input::VectorSearch {} => vector_search::run(),
            Input::Passthrough(values) => passthrough::nonstream_answer(values),
            Input::MessagesCount(values) => messages_count::run(values),
        }
    }
}
