use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::spend_cache_update::cache_read_keys;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    end_user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    tags: Option<Vec<Json>>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    response_cost: Option<Decimal>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let keys = cache_read_keys(
            input.user_id.as_deref(),
            input.end_user_id.as_deref(),
            input.team_id.as_deref(),
            input.tags.as_deref(),
            input.response_cost,
        );
        serde_json::to_value(keys).or_value_error()
    }
}

mod tests;
