use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

mod insert;
pub mod model;
mod validate;

mod tests;

use model::{ConfigName, ConfigRow, FinOpsUsageRow, KeyMetadataRow, OrganizationRow};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinOpsSeed {
    #[serde(default)]
    pub organizations: Vec<OrganizationRow>,
    #[serde(default)]
    pub key_metadata: Vec<KeyMetadataRow>,
    #[serde(default)]
    pub daily_usage: Vec<FinOpsUsageRow>,
    #[serde(default)]
    pub config_rows: Vec<ConfigRow>,
}

#[derive(Debug, Error)]
pub enum FinOpsFixtureError {
    #[error("finops fixture {list} has {count} rows, more than the limit of {limit}")]
    TooManyRows {
        list: &'static str,
        count: usize,
        limit: usize,
    },
    #[error("finops organization row {row} repeats an organization_id")]
    DuplicateOrganization { row: usize },
    #[error("finops organization row {row} references missing budget {budget_id}")]
    MissingBudget { row: usize, budget_id: String },
    #[error("finops organization row {row} field {field} references missing user {user_id}")]
    MissingUser {
        row: usize,
        field: &'static str,
        user_id: String,
    },
    #[error("finops key metadata row {row} references no fixture key")]
    UnknownKeyMetadata { row: usize },
    #[error("finops key metadata row {row} repeats an earlier key target")]
    DuplicateKeyMetadata { row: usize },
    #[error(
        "finops key metadata row {row} field {field} references missing organization {organization_id}"
    )]
    MissingOrganization {
        row: usize,
        field: &'static str,
        organization_id: String,
    },
    #[error("finops key metadata row {row} sets team_organization_id but its key has no team")]
    KeyMetadataWithoutTeam { row: usize },
    #[error("finops key metadata row {row} references missing team {team_id}")]
    MissingTeam { row: usize, team_id: String },
    #[error("finops key metadata rows assign team {team_id} conflicting organizations")]
    ConflictingTeamOrganization { team_id: String },
    #[error("finops daily usage row {row} repeats an id")]
    DuplicateDailyUsageId { row: usize },
    #[error("finops daily usage row {row} references missing user {user_id}")]
    MissingUsageUser { row: usize, user_id: String },
    #[error("finops daily usage row {row} references no fixture key")]
    UnknownUsageKey { row: usize },
    #[error("finops daily usage row {row} orphan_hash is not a SHA256 hex token")]
    InvalidOrphanHash { row: usize },
    #[error("finops daily usage row {row} field {field} is not a valid timestamp")]
    InvalidTimestamp { row: usize, field: &'static str },
    #[error("finops daily usage row {row} repeats a non-null unique tuple")]
    DuplicateUsageTuple { row: usize },
    #[error("finops config row {row} repeats config name {name:?}")]
    DuplicateConfigName { row: usize, name: ConfigName },
    #[error("finops fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
}

pub async fn seed(client: &Client, finops: &FinOpsSeed) -> Result<(), FinOpsFixtureError> {
    insert::organizations(client, &finops.organizations).await?;
    insert::key_metadata(client, &finops.key_metadata).await?;
    insert::daily_usage(client, &finops.daily_usage).await?;
    insert::config_rows(client, &finops.config_rows).await
}
