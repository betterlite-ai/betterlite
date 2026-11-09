mod calls;
mod failures;
mod policies;

use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    UserBlocks(calls::UserBlocksCall),
    SystemPrompt(calls::SystemPromptCall),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::UserBlocks(call) => calls::user_blocks(call),
            Input::SystemPrompt(call) => calls::system_prompt(call),
        }
    }
}
