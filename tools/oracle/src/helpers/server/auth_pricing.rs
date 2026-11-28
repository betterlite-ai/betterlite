mod fixtures;
mod tests;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fixtures::{Input, answer};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answers = Vec::new();
        for call in &input.calls {
            answers.push(match runtime.block_on(answer(call)) {
                Ok(value) => json!({"ok": true, "value": value}),
                Err(_failure) => json!({"ok": false}),
            });
        }
        Ok(Value::Array(answers))
    }
}
