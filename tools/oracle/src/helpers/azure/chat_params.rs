use serde_json::json;

use crate::helpers::{Helper, HelperResult};

use input::Input;
use reports::{
    alias_models, gpt5_models, invalid_input, mapping_steps, o_series_models, runtime,
    stream_steps, transform_steps,
};

mod fixture;
mod input;
mod reports;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = runtime().map_err(|error| invalid_input(error.to_string()))?;
        match input {
            Input::Gpt5AliasCatalog(call) => Ok(json!({"models": alias_models(&runtime, call)})),
            Input::NativeStream(call) => Ok(json!({"steps": stream_steps(&runtime, call)})),
            Input::Gpt5Allowlist(call) => Ok(json!({"models": gpt5_models(&runtime, call)})),
            Input::OSeriesAllowlist(call) => Ok(json!({"models": o_series_models(&runtime, call)})),
            Input::ParameterMapping(call) => Ok(json!({"steps": mapping_steps(&runtime, call)})),
            Input::RequestTransform(call) => Ok(json!({"steps": transform_steps(&runtime, call)?})),
        }
    }
}
