use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use tokio::runtime::Builder;

mod calls;
mod rows;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Get(Get),
    Bulk(Bulk),
    Merge(Merge),
    Delete(Delete),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Get {
    salt_key: String,
    user_id: String,
    server_id: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bulk {
    salt_key: String,
    user_id: String,
    server_ids: Vec<String>,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Merge {
    salt_key: String,
    user_id: String,
    server_id: String,
    updates: BTreeMap<String, String>,
    allowed_names: Vec<String>,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Delete {
    salt_key: String,
    user_id: String,
    server_id: String,
    rows: Vec<JsonData>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        runtime.block_on(async {
            match input {
                Input::Get(call) => calls::get(call).await,
                Input::Bulk(call) => calls::bulk(call).await,
                Input::Merge(call) => calls::merge(call).await,
                Input::Delete(call) => calls::delete(call).await,
            }
        })
    }
}
