use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::access_windows::reservation::reserved_deployments;
use betterlite_router::access_windows::team::filter_team_based_models;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_catalog::types::Authority;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// The two policy layers B322 builds: the caller's team narrows the pool, and active reservations empty it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Team,
    Reserved,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model: String,
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub proxy_admin: bool,
    #[serde(default)]
    pub now_unix_millis: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub model_name: String,
    pub id: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team_public_model_name: Option<String>,
    #[serde(default)]
    pub access_windows: Option<Vec<Window>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: String,
    pub end: String,
    pub timezone: String,
    pub team_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub kept: Vec<String>,
    pub error: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let deployments = input
        .deployments
        .iter()
        .map(spec)
        .collect::<Result<Vec<_>, OperationError>>()?;
    let result = filtered(input, deployments)?;
    let output = match result {
        Ok(kept) => Output {
            kept: kept
                .iter()
                .map(|deployment| deployment.id().unwrap_or_default().to_owned())
                .collect(),
            error: None,
        },
        Err(error) => Output {
            kept: Vec::new(),
            error: Some(error.to_string()),
        },
    };
    Ok(ProbeOutput::AccessWindows(output))
}

fn filtered(
    input: &Input,
    deployments: Vec<DeploymentSpec>,
) -> Result<Result<Vec<DeploymentSpec>, CatalogLookupError>, OperationError> {
    match input.mode {
        Mode::Team => filter_team_based_models(
            deployments,
            Authority {
                team_id: input.team_id.as_deref(),
                proxy_admin: input.proxy_admin,
            },
            &input.model,
        ),
        Mode::Reserved => {
            let millis = input
                .now_unix_millis
                .ok_or_else(|| invalid("the reserved mode needs now_unix_millis"))?;
            let now = DateTime::<Utc>::from_timestamp_millis(millis)
                .ok_or_else(|| invalid("now_unix_millis is out of range"))?;
            let database = TimeZoneDatabase::from_env();
            reserved_deployments(
                &input.model,
                &deployments,
                input.team_id.as_deref(),
                now,
                &database,
            )
        }
    }
}

fn spec(deployment: &Deployment) -> Result<DeploymentSpec, OperationError> {
    let mut model_info =
        JsonObject::from_iter([("id".to_owned(), Json::Text(deployment.id.clone()))]);
    if let Some(team_id) = &deployment.team_id {
        model_info.insert("team_id".to_owned(), Json::Text(team_id.clone()));
    }
    if let Some(public) = &deployment.team_public_model_name {
        model_info.insert(
            "team_public_model_name".to_owned(),
            Json::Text(public.clone()),
        );
    }
    if let Some(windows) = &deployment.access_windows {
        model_info.insert(
            "access_windows".to_owned(),
            Json::List(windows.iter().map(window_json).collect()),
        );
    }
    DeploymentSpec::new(
        deployment.model_name.clone(),
        JsonObject::new(),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .map_err(|error| invalid(&error.to_string()))
}

fn window_json(window: &Window) -> Json {
    Json::Object(JsonObject::from_iter([
        ("start".to_owned(), Json::Text(window.start.clone())),
        ("end".to_owned(), Json::Text(window.end.clone())),
        ("timezone".to_owned(), Json::Text(window.timezone.clone())),
        (
            "team_ids".to_owned(),
            Json::List(
                window
                    .team_ids
                    .iter()
                    .map(|id| Json::Text(id.clone()))
                    .collect(),
            ),
        ),
    ]))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
