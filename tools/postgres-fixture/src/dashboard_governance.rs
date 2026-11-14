mod insert;
pub mod model;
mod tests;
mod validate;

use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

use crate::seed::FixtureSeed;

use self::model::{GuardrailIndexRow, GuardrailMetricRow, ToolIndexRow, ToolRow, UsageUnitRow};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardGovernanceSeed {
    #[serde(default)]
    pub metrics: Vec<GuardrailMetricRow>,
    #[serde(default)]
    pub usage_units: Vec<UsageUnitRow>,
    #[serde(default)]
    pub guardrail_index: Vec<GuardrailIndexRow>,
    #[serde(default)]
    pub tool_index: Vec<ToolIndexRow>,
    #[serde(default)]
    pub tools: Vec<ToolRow>,
}

#[derive(Debug, Error)]
pub enum DashboardGovernanceError {
    #[error("dashboard_governance needs a dashboard seed, which installs the pinned schema")]
    MissingDashboard,
    #[error("dashboard_governance {list} has {count} rows, more than the limit of {limit}")]
    TooManyRows {
        list: &'static str,
        count: usize,
        limit: usize,
    },
    #[error(
        "dashboard_governance {list} row {row} field {field} is {bytes} bytes, over the limit of {limit}"
    )]
    TextTooLong {
        list: &'static str,
        row: usize,
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    #[error("dashboard_governance {list} row {row} repeats a database identity")]
    DuplicateIdentity { list: &'static str, row: usize },
    #[error(
        "dashboard_governance {list} row {row} has key_index {key_index}, but only {keys} keys are seeded"
    )]
    UnknownKey {
        list: &'static str,
        row: usize,
        key_index: usize,
        keys: usize,
    },
    #[error("dashboard_governance {list} row {row} references missing team {team_id}")]
    MissingTeam {
        list: &'static str,
        row: usize,
        team_id: String,
    },
    #[error(
        "dashboard_governance {list} row {row} names log {request_id}, which dashboard.logs does not seed"
    )]
    UnknownLog {
        list: &'static str,
        row: usize,
        request_id: String,
    },
    #[error(
        "dashboard_governance {list} row {row} counter {field} is {value}, but counters cannot be negative"
    )]
    NegativeCounter {
        list: &'static str,
        row: usize,
        field: &'static str,
        value: i64,
    },
    #[error("dashboard_governance {list} row {row} failed to insert")]
    Insert {
        list: &'static str,
        row: usize,
        #[source]
        source: tokio_postgres::Error,
    },
}

pub fn validate(
    governance: &DashboardGovernanceSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    let Some(dashboard_seed) = seed.dashboard.as_ref() else {
        return Err(DashboardGovernanceError::MissingDashboard);
    };
    validate::rows(governance, seed, dashboard_seed)
}

pub async fn seed(
    client: &Client,
    governance: &DashboardGovernanceSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    validate(governance, seed)?;
    insert::seed(client, governance, seed).await
}
