use std::collections::{BTreeMap, BTreeSet};

use secrecy::ExposeSecret as _;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::DashboardGovernanceError;
use super::DashboardGovernanceSeed;
use crate::seed::FixtureSeed;

mod guardrail_index;
mod measurements;
mod tool_index;
mod tools;
mod usage_units;

// both fixture gateways get this instant in their naive timestamp columns, never a clock read
static CREATED_AT: &str = "2026-01-01T00:00:00Z";

const USER_TEAMS_UPDATE: &str =
    "UPDATE \"LiteLLM_UserTable\" SET \"teams\" = $1 WHERE \"user_id\" = $2";

pub(super) async fn seed(
    client: &Client,
    governance: &DashboardGovernanceSeed,
    fixture: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    update_user_teams(client, fixture).await?;
    measurements::insert(client, &governance.metrics).await?;
    usage_units::insert(client, &governance.usage_units, fixture).await?;
    guardrail_index::insert(client, &governance.guardrail_index).await?;
    tool_index::insert(client, &governance.tool_index).await?;
    tools::insert(client, &governance.tools, fixture).await
}

pub(super) async fn execute(
    client: &Client,
    list: &'static str,
    row: usize,
    sql: &str,
    values: &[&(dyn ToSql + Sync)],
) -> Result<(), DashboardGovernanceError> {
    client
        .execute(sql, values)
        .await
        .map_err(|source| DashboardGovernanceError::Insert { list, row, source })?;
    Ok(())
}

async fn update_user_teams(
    client: &Client,
    fixture: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    let mut teams: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for membership in &fixture.memberships {
        teams
            .entry(membership.user_id.as_str())
            .or_default()
            .insert(membership.team_id.as_str());
    }
    for (row, user) in fixture.users.iter().enumerate() {
        let member_of = teams
            .get(user.user_id.as_str())
            .map_or_else(Vec::<String>::new, |ids| {
                ids.iter().map(|team_id| String::from(*team_id)).collect()
            });
        let values: &[&(dyn ToSql + Sync)] = &[&member_of, &user.user_id];
        execute(client, "memberships", row, USER_TEAMS_UPDATE, values).await?;
    }
    Ok(())
}

pub(super) fn measured_key_hash(
    list: &'static str,
    row: usize,
    index: usize,
    fixture: &FixtureSeed,
) -> Result<String, DashboardGovernanceError> {
    let Some(key) = fixture.keys.get(index) else {
        return Err(DashboardGovernanceError::UnknownKey {
            list,
            row,
            key_index: index,
            keys: fixture.keys.len(),
        });
    };
    Ok(FixtureSeed::key_hash(key.token.expose_secret()))
}
