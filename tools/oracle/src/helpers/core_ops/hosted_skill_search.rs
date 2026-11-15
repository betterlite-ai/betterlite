mod calls;
mod input;
mod scripted;
mod values;

use input::Input;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let effects = scripted::Effects::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| values::invalid_input(error.to_string()))?;
        match input.function.as_str() {
            "text" => calls::text_value(input),
            "rank" | "failures" => runtime.block_on(calls::index_value(input, &effects)),
            "search" => runtime.block_on(calls::search_value(input, &effects)),
            "hosted" => runtime.block_on(calls::hosted_value(input, &effects)),
            other => Err(values::invalid_input(format!("unknown function: {other}"))),
        }
    }
}
