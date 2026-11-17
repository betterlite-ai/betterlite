mod chat;
mod fixtures;
mod input;
mod listing;
mod wire;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Case;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        match input.op {
            input::Operation::Chat => chat::run(input),
            input::Operation::LemonadeModels => listing::lemonade_models(&runtime, input),
            input::Operation::LemonadeReply => listing::lemonade_reply(input),
            input::Operation::OllamaModels => listing::ollama_models(input),
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
