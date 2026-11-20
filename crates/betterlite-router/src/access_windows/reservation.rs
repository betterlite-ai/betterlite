use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::access_windows::window::{
    ModelAccessWindow, WindowError, is_window_active, parse_access_windows, reserved_message,
};
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_identity::spec::DeploymentSpec;

// filter_reserved_deployments' answer: the deployments no active window blocks, and the first window that blocked one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservations {
    pub deployments: Vec<DeploymentSpec>,
    pub blocking_window: Option<ModelAccessWindow>,
}

// _reservation_blocking_window: the first active window that does not name the caller's team blocks the deployment.
fn blocking_window(
    deployment: &DeploymentSpec,
    team_id: Option<&str>,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<Option<ModelAccessWindow>, WindowError> {
    let mut active = Vec::new();
    for window in parse_access_windows(&deployment.model_info, database)? {
        if is_window_active(&window, now)? {
            active.push(window);
        }
    }
    if active.is_empty() {
        return Ok(None);
    }
    let mine = team_id.is_some_and(|team| {
        active
            .iter()
            .any(|window| window.team_ids.iter().any(|id| id == team))
    });
    if mine {
        return Ok(None);
    }
    Ok(active.into_iter().next())
}

// filter_reserved_deployments: a deployment is kept unless one of its active windows names another team.
pub fn filter_reserved_deployments(
    deployments: &[DeploymentSpec],
    team_id: Option<&str>,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<Reservations, WindowError> {
    let mut kept = Vec::with_capacity(deployments.len());
    let mut blocking = None;
    for deployment in deployments {
        let Some(window) = blocking_window(deployment, team_id, now, database)? else {
            kept.push(deployment.clone());
            continue;
        };
        if blocking.is_none() {
            blocking = Some(window);
        }
    }
    Ok(Reservations {
        deployments: kept,
        blocking_window: blocking,
    })
}

// Router._filter_reserved_deployments: a pool an active reservation emptied earns the refusal naming its window.
pub fn reserved_deployments(
    model: &str,
    deployments: &[DeploymentSpec],
    team_id: Option<&str>,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
    let reservations =
        filter_reserved_deployments(deployments, team_id, now, database).map_err(|error| {
            CatalogLookupError::Reserved {
                message: format!(
                    "Deployment {model} has an invalid model_info.access_windows: {error}"
                ),
            }
        })?;
    match reservations.blocking_window {
        Some(window) if reservations.deployments.is_empty() => Err(CatalogLookupError::Reserved {
            message: reserved_message(model, &window),
        }),
        _ => Ok(reservations.deployments),
    }
}
