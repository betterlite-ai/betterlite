use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_budget_source::{
    TeamMemberBudgetSource, member_budget_source, team_default_budget_id,
};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    DefaultId(DefaultIdInput),
    Source(SourceInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultIdInput {
    metadata: Vec<Option<BTreeMap<String, JsonData>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceInput {
    pairs: Vec<(Option<String>, Option<String>)>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DefaultId(fields) => {
                let ids: Vec<Option<&str>> = fields
                    .metadata
                    .iter()
                    .map(|metadata| team_default_budget_id(metadata.as_ref()))
                    .collect();
                Ok(serde_json::json!(ids))
            }
            Input::Source(fields) => {
                let labels: Vec<TeamMemberBudgetSource> = fields
                    .pairs
                    .iter()
                    .map(|(budget_id, default_id)| {
                        member_budget_source(budget_id.as_deref(), default_id.as_deref())
                    })
                    .collect();
                Ok(serde_json::json!(labels))
            }
        }
    }
}
