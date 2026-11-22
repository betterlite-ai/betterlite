mod chat;
mod embed;
mod fixtures;
mod input;
mod wire;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Case;

    fn run(input: &Self::Input) -> HelperResult {
        match input.op {
            input::Operation::Chat => chat::run(input),
            input::Operation::Embedding => embed::run(input),
        }
    }
}

pub(super) fn input_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
