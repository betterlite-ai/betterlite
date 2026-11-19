use betterlite_config::sections::access_windows as config;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::access_windows::reservation::reserved_deployments;
use betterlite_router::access_windows::window::{
    ModelAccessWindow, WindowError, parse_access_windows,
};
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use thiserror::Error;

mod tests;

// The injected clock and zone database the reservation filter reads, resolved once per request.
#[derive(Clone, Copy)]
pub struct Windows<'context> {
    pub now: DateTime<Utc>,
    pub database: &'context TimeZoneDatabase,
}

impl Windows<'_> {
    // Router._filter_reserved_deployments: the pool after its active reservations, or the refusal naming the blocking window.
    pub fn reserved(
        &self,
        model: &str,
        deployments: Vec<DeploymentSpec>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        reserved_deployments(model, deployments, team_id, self.now, self.database)
    }
}

// The access_windows each model_list entry configures, validated at load so a bad window stops startup instead of every request.
pub fn deployment_windows(
    section: &config::Settings,
    database: &TimeZoneDatabase,
) -> Result<Vec<Option<Vec<ModelAccessWindow>>>, AccessWindowsError> {
    let mut deployments = Vec::with_capacity(section.deployments.len());
    for (index, windows) in section.deployments.iter().enumerate() {
        let Some(windows) = windows else {
            deployments.push(None);
            continue;
        };
        let parsed = parse_access_windows(&model_info(windows), database)
            .map_err(|source| AccessWindowsError { index, source })?;
        deployments.push(Some(parsed));
    }
    Ok(deployments)
}

fn model_info(windows: &[config::WindowConfig]) -> JsonObject {
    let configured: Vec<Json> = windows.iter().map(window_json).collect();
    JsonObject::from_iter([("access_windows".to_owned(), Json::List(configured))])
}

fn window_json(window: &config::WindowConfig) -> Json {
    let mut entries = JsonObject::new();
    for (key, value) in [
        ("start", &window.start),
        ("end", &window.end),
        ("timezone", &window.timezone),
    ] {
        if let Some(text) = value {
            entries.insert(key.to_owned(), Json::Text(text.clone()));
        }
    }
    if let Some(ids) = &window.team_ids {
        entries.insert(
            "team_ids".to_owned(),
            Json::List(ids.iter().map(|id| Json::Text(id.clone())).collect()),
        );
    }
    Json::Object(entries)
}

#[derive(Debug, Error)]
#[error("model_list.{index}.model_info.access_windows: {source}")]
pub struct AccessWindowsError {
    pub index: usize,
    #[source]
    pub source: WindowError,
}
