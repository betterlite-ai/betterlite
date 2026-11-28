use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::user_search_predicate::user_search_where;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    search: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let Some(predicate) = user_search_where(input.search.as_deref()) else {
            return Ok(serde_json::json!({}));
        };
        Ok(serde_json::to_value(predicate)?)
    }
}
