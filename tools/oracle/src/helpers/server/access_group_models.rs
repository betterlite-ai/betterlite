use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_server::access_group_models::{
    ModelSyncError, config_sourced_sibling, served_by_a_config_deployment,
};
use betterlite_server::model_visibility::testing::{FixtureError, Row as SnapshotRow, Snapshot};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error("the config_sourced_sibling function needs a deployment_id")]
    MissingDeploymentId,
    #[error("the served_by_a_config_deployment function needs a model_name")]
    MissingModelName,
    #[error(transparent)]
    Lookup(#[from] ModelSyncError),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Predicate {
    ConfigSourcedSibling,
    ServedByAConfigDeployment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    model_name: String,
    litellm_params: JsonObject,
    #[serde(default)]
    model_info: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Predicate,
    model_list: Vec<Row>,
    #[serde(default)]
    deployment_id: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    model_id: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

// Router(model_list=...): the case's rows under the catalog indices the lookups read.
fn answer(input: &Input) -> Result<Value, Failure> {
    let infos: Vec<Json> = input
        .model_list
        .iter()
        .map(|row| Json::Object(row.model_info.clone()))
        .collect();
    let rows: Vec<SnapshotRow<'_>> = input
        .model_list
        .iter()
        .zip(&infos)
        .map(|(row, model_info)| SnapshotRow {
            model_name: &row.model_name,
            litellm_params: &row.litellm_params,
            model_info,
        })
        .collect();
    let fixture = Snapshot::build(&rows, &JsonObject::new())?;
    match input.function {
        Predicate::ConfigSourcedSibling => {
            let Some(deployment_id) = input.deployment_id.as_deref() else {
                return Err(Failure::MissingDeploymentId);
            };
            let found = config_sourced_sibling(&fixture.catalog, deployment_id, &input.model_id)
                .map_err(ModelSyncError::from)?;
            Ok(json!({"result": found}))
        }
        Predicate::ServedByAConfigDeployment => {
            let Some(model_name) = input.model_name.as_deref() else {
                return Err(Failure::MissingModelName);
            };
            let found =
                served_by_a_config_deployment(Some(&fixture.catalog), model_name, &input.model_id)
                    .map_err(ModelSyncError::from)?;
            Ok(json!({"result": found}))
        }
    }
}
