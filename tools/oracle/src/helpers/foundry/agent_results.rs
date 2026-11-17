use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Map, Value};
use tokio::runtime::Builder;

mod answers;
mod input;
mod tests;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answer = Map::new();
        if let Some(calls) = &input.extractions {
            answer.insert(String::from("extractions"), answers::extractions(calls)?);
        }
        if let Some(calls) = &input.annotations {
            answer.insert(String::from("annotations"), answers::annotations(calls)?);
        }
        if let Some(calls) = &input.replies {
            answer.insert(
                String::from("replies"),
                runtime.block_on(answers::replies(calls))?,
            );
        }
        if let Some(calls) = &input.streams {
            answer.insert(
                String::from("streams"),
                runtime.block_on(answers::streams(calls))?,
            );
        }
        if let Some(calls) = &input.requests {
            answer.insert(
                String::from("requests"),
                runtime.block_on(answers::requests(calls))?,
            );
        }
        Ok(Value::Object(answer))
    }
}
