mod calls;
mod input;
mod scripted;
mod values;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let effects = scripted::Effects::default();
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| values::invalid_input(error.to_string()))?;
        match input.function.as_str() {
            "text" => calls::text_value(input),
            "rank" | "failures" => {
                runtime.block_on(calls::ranked_value(input, &effects, calls::Ranking::Index))
            }
            "search" => {
                runtime.block_on(calls::ranked_value(input, &effects, calls::Ranking::Search))
            }
            "hosted" => runtime.block_on(calls::hosted_value(input, &effects)),
            other => Err(values::invalid_input(format!("unknown function: {other}"))),
        }
    }
}
