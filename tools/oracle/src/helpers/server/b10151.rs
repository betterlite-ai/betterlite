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
            Input::DefaultId(fields) => Ok(default_ids(fields)),
            Input::Source(fields) => Ok(sources(fields)),
        }
    }
}

fn default_ids(fields: &DefaultIdInput) -> serde_json::Value {
    let ids: Vec<Option<&str>> = fields
        .metadata
        .iter()
        .map(|metadata| team_default_budget_id(metadata.as_ref()))
        .collect();
    serde_json::json!(ids)
}

fn sources(fields: &SourceInput) -> serde_json::Value {
    let labels: Vec<TeamMemberBudgetSource> = fields
        .pairs
        .iter()
        .map(|(budget_id, default_id)| {
            member_budget_source(budget_id.as_deref(), default_id.as_deref())
        })
        .collect();
    serde_json::json!(labels)
}
