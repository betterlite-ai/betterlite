use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    model_list: Vec<JsonObject>,
    request: serde_json::Value,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        betterlite_server::cost_estimate_route::testing::quote(&input.model_list, &input.request)
            .or_value_error()
    }
}
