use betterlite_prompts::message_content::Message;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod plan;
mod sanitize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    PlanFileScans { messages: Vec<Message> },
    SanitizeRequest(sanitize::SanitizeRequest),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PlanFileScans { messages } => plan::plan(messages),
            Input::SanitizeRequest(request) => sanitize::sanitize(request),
        }
    }
}
