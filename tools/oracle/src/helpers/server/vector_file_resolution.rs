mod answer;
mod input;
mod script;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| script::failure(error.to_string()))?;
        runtime.block_on(async {
            let mut answers: Vec<Value> = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                answers.push(answer::answer(call).await?);
            }
            Ok(json!(answers))
        })
    }
}
