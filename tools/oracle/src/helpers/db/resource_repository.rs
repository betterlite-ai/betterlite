use betterlite_db::resource_repository::config::decode_config_json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    raw: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let decoded = decode_config_json(input.raw.as_bytes()).or_value_error()?;
        serde_json::to_value(decoded).or_value_error()
    }
}
