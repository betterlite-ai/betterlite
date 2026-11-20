use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::object_permissions::entries::mcp_tool_permission_entries;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    raw: JsonData,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let entries: BTreeMap<String, BTreeSet<String>> =
            mcp_tool_permission_entries(Some(&input.raw)).or_value_error()?;
        Ok(serde_json::to_value(entries)?)
    }
}

mod tests;
