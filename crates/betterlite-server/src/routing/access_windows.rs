use betterlite_config::sections::access_windows as config;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::access_windows::window::{
    ModelAccessWindow, WindowError, parse_access_windows,
};
use jiff::tz::TimeZoneDatabase;
use thiserror::Error;

mod tests;

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
        let configured = JsonObject::from_iter([(
            "access_windows".to_owned(),
            Json::List(windows.iter().map(window_json).collect()),
        )]);
        let parsed = parse_access_windows(&configured, database)
            .map_err(|source| AccessWindowsError { index, source })?;
        deployments.push(Some(parsed));
    }
    Ok(deployments)
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
