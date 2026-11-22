use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_server::model_group_data::budgets::{budget_or_none, budget_response, with_budget};
use betterlite_server::model_group_data::deployments::{
    add_access_group_to_deployment, readable_model_info_or_raise, remove_group_from_deployment,
};
use betterlite_server::model_group_data::discovery::validate_models_exist;
use betterlite_server::model_group_data::errors::GroupDataError;
use betterlite_server::model_group_data::ports::ModelNameSource;
use betterlite_server::model_group_data::reload::raise_if_degraded_serving;
use betterlite_server::model_group_data::types::{
    AccessGroupInfo, GroupBudgetRow, ModelAccessGroupBudget,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    FlattenedBudget(BudgetRecordCall),
    WithBudget(WithBudgetCall),
    BudgetOrNone(RowCall),
    BudgetResponse(ResponseCall),
    ValidateModelsExist(ValidateCall),
    AddAccessGroupToDeployment(GroupCall),
    #[serde(rename = "remove_access_group_from_deployment")]
    RemoveDeploymentGroup(GroupCall),
    ReadableModelInfoOrRaise(ReadableCall),
    #[serde(rename = "raise_http_if_reload_degraded_serving")]
    RaiseDegradedServing(RaiseCall),
}

#[derive(Debug, Deserialize)]
struct BudgetRecordCall {
    record: ModelAccessGroupBudget,
}

#[derive(Debug, Deserialize)]
struct WithBudgetCall {
    info: AccessGroupInfo,
    row: Option<GroupBudgetRow>,
}

#[derive(Debug, Deserialize)]
struct RowCall {
    row: Option<GroupBudgetRow>,
}

#[derive(Debug, Deserialize)]
struct ResponseCall {
    access_group: String,
    row: Option<GroupBudgetRow>,
}

#[derive(Debug, Deserialize)]
struct ValidateCall {
    model_names: Vec<String>,
    router_names: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct GroupCall {
    model_info: JsonObject,
    access_group: String,
}

#[derive(Debug, Deserialize)]
struct ReadableCall {
    model_id: String,
    model_info: Option<Json>,
}

#[derive(Debug, Deserialize)]
struct WrittenInput {
    id: String,
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
struct RaiseCall {
    before: Vec<String>,
    written: Vec<WrittenInput>,
    access_group: String,
    still_desired: Option<Vec<String>>,
    live_after: Vec<String>,
}

struct SuppliedNames(Option<Vec<String>>);

impl ModelNameSource for SuppliedNames {
    fn model_names(&self) -> Result<Option<Vec<String>>, LookupError> {
        Ok(self.0.clone())
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer_call(call));
        }
        Ok(json!({ "calls": answers }))
    }
}

fn answer_call(call: &Call) -> Value {
    match call {
        Call::FlattenedBudget(call) => json!({ "ok": true, "value": call.record }),
        Call::WithBudget(call) => {
            json!({ "ok": true, "value": with_budget(&call.info, call.row.as_ref()) })
        }
        Call::BudgetOrNone(call) => {
            json!({ "ok": true, "value": budget_or_none(call.row.as_ref()) })
        }
        Call::BudgetResponse(call) => json!({
            "ok": true,
            "value": budget_response(&call.access_group, call.row.as_ref()),
        }),
        Call::ValidateModelsExist(call) => validate_answer(call),
        Call::AddAccessGroupToDeployment(call) => add_answer(call),
        Call::RemoveDeploymentGroup(call) => remove_answer(call),
        Call::ReadableModelInfoOrRaise(call) => readable_answer(call),
        Call::RaiseDegradedServing(call) => raise_answer(call),
    }
}

fn validate_answer(call: &ValidateCall) -> Value {
    let source = SuppliedNames(call.router_names.clone());
    match validate_models_exist(&call.model_names, Some(&source)) {
        Ok((all_valid, missing)) => json!({
            "ok": true,
            "value": { "all_valid": all_valid, "missing": missing },
        }),
        Err(error) => failure(&error),
    }
}

fn add_answer(call: &GroupCall) -> Value {
    let mut model_info = call.model_info.clone();
    match add_access_group_to_deployment(&mut model_info, &call.access_group) {
        Ok(modified) => json!({
            "ok": true,
            "value": { "model_info": model_info, "modified": modified },
        }),
        Err(error) => failure(&error),
    }
}

fn remove_answer(call: &GroupCall) -> Value {
    let mut model_info = call.model_info.clone();
    match remove_group_from_deployment(&mut model_info, &call.access_group) {
        Ok(modified) => json!({
            "ok": true,
            "value": { "model_info": model_info, "modified": modified },
        }),
        Err(error) => failure(&error),
    }
}

fn readable_answer(call: &ReadableCall) -> Value {
    match readable_model_info_or_raise(&call.model_id, call.model_info.as_ref()) {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => failure(&error),
    }
}

fn raise_answer(call: &RaiseCall) -> Value {
    let before: BTreeSet<String> = call.before.iter().cloned().collect();
    let still_desired = call
        .still_desired
        .as_ref()
        .map(|ids| ids.iter().cloned().collect::<BTreeSet<String>>());
    let live_after: BTreeSet<String> = call.live_after.iter().cloned().collect();
    let written: Vec<(String, JsonObject)> = call
        .written
        .iter()
        .map(|pair| (pair.id.clone(), pair.model_info.clone()))
        .collect();
    match raise_if_degraded_serving(
        &before,
        &written,
        &call.access_group,
        still_desired.as_ref(),
        Some(&live_after),
    ) {
        Ok(()) => json!({ "ok": true, "value": null }),
        Err(error) => failure(&error),
    }
}

// helper cases compare an error's HTTP status, never its class or message
fn failure(error: &GroupDataError) -> Value {
    let status = match error {
        GroupDataError::Http(http) => Some(http.status.as_u16()),
        GroupDataError::ValueError { .. }
        | GroupDataError::Runtime { .. }
        | GroupDataError::Database(_)
        | GroupDataError::Catalog(_)
        | GroupDataError::Row(_) => None,
    };
    json!({ "ok": false, "status": status })
}
