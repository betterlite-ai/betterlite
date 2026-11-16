use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use input::Input;
use reports::{
    Series, alias_models, allowlist_models, invalid_input, mapping_steps, stream_steps,
    transform_steps,
};

mod fixture;
mod input;
mod reports;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        match input {
            Input::Gpt5AliasCatalog(call) => Ok(json!({"models": alias_models(&runtime, call)})),
            Input::NativeStream(call) => Ok(json!({"steps": stream_steps(&runtime, call)})),
            Input::Gpt5Allowlist(call) => Ok(json!({
                "models": allowlist_models(&runtime, call, Series::Gpt5),
            })),
            Input::OSeriesAllowlist(call) => Ok(json!({
                "models": allowlist_models(&runtime, call, Series::OSeries),
            })),
            Input::ParameterMapping(call) => Ok(json!({"steps": mapping_steps(&runtime, call)})),
            Input::RequestTransform(call) => Ok(json!({"steps": transform_steps(&runtime, call)?})),
        }
    }
}
