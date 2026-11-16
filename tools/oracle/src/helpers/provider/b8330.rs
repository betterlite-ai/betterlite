mod chat;
mod embed;
mod fixture;
mod input;
mod stream;
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
        match input.op.as_str() {
            "chat" => chat::run(&runtime, input),
            "embedding" => embed::run(&runtime, input),
            "oci_stream" => stream::run(&runtime, input),
            other => Err(input_failure(format!(
                "the b8330 helper has no {other} operation"
            ))),
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
