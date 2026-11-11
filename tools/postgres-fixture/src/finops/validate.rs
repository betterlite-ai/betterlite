use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDateTime};
use secrecy::ExposeSecret as _;

use super::FinOpsFixtureError;
use super::FinOpsSeed;
use super::model::{
    ConfigRow, FinOpsUsageRow, KeyMetadataRow, OrganizationRow, UsageApiKey, secret_eq,
};
use crate::seed::FixtureSeed;

const MAX_ROWS: usize = 1024;
const SHA256_HEX_CHARS: usize = 64;

impl FinOpsSeed {
    pub fn validate(&self, fixture: &FixtureSeed) -> Result<(), FinOpsFixtureError> {
        check_row_limits(self)?;
        let users = fixture
            .users
            .iter()
            .map(|row| row.user_id.as_str())
            .collect::<BTreeSet<_>>();
        let budgets = fixture
            .budgets
            .iter()
            .map(|row| row.budget_id.as_str())
            .collect::<BTreeSet<_>>();
        let organizations = organizations_by_id(&self.organizations, &budgets, &users)?;
        key_metadata_targets(&self.key_metadata, fixture, &organizations)?;
        daily_usage_rows(&self.daily_usage, fixture, &users)?;
        config_names(&self.config_rows)
    }
}

fn check_row_limits(seed: &FinOpsSeed) -> Result<(), FinOpsFixtureError> {
    for (list, count) in [
        ("organizations", seed.organizations.len()),
        ("key_metadata", seed.key_metadata.len()),
        ("daily_usage", seed.daily_usage.len()),
        ("config_rows", seed.config_rows.len()),
    ] {
        if count > MAX_ROWS {
            return Err(FinOpsFixtureError::TooManyRows {
                list,
                count,
                limit: MAX_ROWS,
            });
        }
    }
    Ok(())
}

fn organizations_by_id<'organization>(
    organizations: &'organization [OrganizationRow],
    budgets: &BTreeSet<&str>,
    users: &BTreeSet<&str>,
) -> Result<BTreeSet<&'organization str>, FinOpsFixtureError> {
    let mut identities = BTreeSet::new();
    for (row, organization) in organizations.iter().enumerate() {
        if !identities.insert(organization.organization_id.as_str()) {
            return Err(FinOpsFixtureError::DuplicateOrganization { row });
        }
        if !budgets.contains(organization.budget_id.as_str()) {
            return Err(FinOpsFixtureError::MissingBudget {
                row,
                budget_id: organization.budget_id.clone(),
            });
        }
        for (field, user_id) in [
            ("created_by", &organization.created_by),
            ("updated_by", &organization.updated_by),
        ] {
            if !users.contains(user_id.as_str()) {
                return Err(FinOpsFixtureError::MissingUser {
                    row,
                    field,
                    user_id: user_id.clone(),
                });
            }
        }
    }
    Ok(identities)
}

fn key_metadata_targets(
    rows: &[KeyMetadataRow],
    fixture: &FixtureSeed,
    organizations: &BTreeSet<&str>,
) -> Result<(), FinOpsFixtureError> {
    let mut targets = BTreeSet::new();
    let mut teams = BTreeMap::new();
    for (row, metadata) in rows.iter().enumerate() {
        let Some((key_index, key)) = fixture
            .keys
            .iter()
            .enumerate()
            .find(|(_, key)| secret_eq(&key.token, &metadata.token))
        else {
            return Err(FinOpsFixtureError::UnknownKeyMetadata { row });
        };
        if !targets.insert(key_index) {
            return Err(FinOpsFixtureError::DuplicateKeyMetadata { row });
        }
        organization_reference(
            "organization_id",
            metadata.organization_id.as_deref(),
            row,
            organizations,
        )?;
        organization_reference(
            "team_organization_id",
            metadata.team_organization_id.as_deref(),
            row,
            organizations,
        )?;
        check_team_assignment(row, key.team_id.as_deref(), metadata, fixture, &mut teams)?;
    }
    Ok(())
}

fn check_team_assignment(
    row: usize,
    key_team_id: Option<&str>,
    metadata: &KeyMetadataRow,
    fixture: &FixtureSeed,
    teams: &mut BTreeMap<String, String>,
) -> Result<(), FinOpsFixtureError> {
    let Some(team_organization_id) = metadata.team_organization_id.as_deref() else {
        return Ok(());
    };
    let Some(team_id) = key_team_id else {
        return Err(FinOpsFixtureError::KeyMetadataWithoutTeam { row });
    };
    if !fixture.teams.iter().any(|team| team.team_id == team_id) {
        return Err(FinOpsFixtureError::MissingTeam {
            row,
            team_id: team_id.to_owned(),
        });
    }
    if let Some(previous) = teams.insert(team_id.to_owned(), team_organization_id.to_owned())
        && previous != team_organization_id
    {
        return Err(FinOpsFixtureError::ConflictingTeamOrganization {
            team_id: team_id.to_owned(),
        });
    }
    Ok(())
}

fn organization_reference(
    field: &'static str,
    referenced_id: Option<&str>,
    row: usize,
    organizations: &BTreeSet<&str>,
) -> Result<(), FinOpsFixtureError> {
    if let Some(organization_id) = referenced_id
        && !organizations.contains(organization_id)
    {
        return Err(FinOpsFixtureError::MissingOrganization {
            row,
            field,
            organization_id: organization_id.to_owned(),
        });
    }
    Ok(())
}

fn daily_usage_rows(
    rows: &[FinOpsUsageRow],
    fixture: &FixtureSeed,
    users: &BTreeSet<&str>,
) -> Result<(), FinOpsFixtureError> {
    let mut identities = BTreeSet::new();
    let mut tuples = BTreeSet::new();
    for (row_index, row) in rows.iter().enumerate() {
        if !identities.insert(row.id.as_str()) {
            return Err(FinOpsFixtureError::DuplicateDailyUsageId { row: row_index });
        }
        if let Some(user_id) = row.user_id.as_deref()
            && !users.contains(user_id)
        {
            return Err(FinOpsFixtureError::MissingUsageUser {
                row: row_index,
                user_id: user_id.to_owned(),
            });
        }
        let api_key = resolve_api_key(row, row_index, fixture)?;
        check_timestamps(row, row_index)?;
        if let Some(tuple) = unique_tuple(row, api_key)
            && !tuples.insert(tuple)
        {
            return Err(FinOpsFixtureError::DuplicateUsageTuple { row: row_index });
        }
    }
    Ok(())
}

fn resolve_api_key(
    row: &FinOpsUsageRow,
    row_index: usize,
    fixture: &FixtureSeed,
) -> Result<String, FinOpsFixtureError> {
    match &row.api_key {
        UsageApiKey::Key(token) => {
            if !fixture.keys.iter().any(|key| secret_eq(&key.token, token)) {
                return Err(FinOpsFixtureError::UnknownUsageKey { row: row_index });
            }
            Ok(FixtureSeed::key_hash(token.expose_secret()))
        }
        UsageApiKey::OrphanHash(hash) => {
            if hash.len() != SHA256_HEX_CHARS || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(FinOpsFixtureError::InvalidOrphanHash { row: row_index });
            }
            Ok(hash.clone())
        }
    }
}

fn unique_tuple(row: &FinOpsUsageRow, api_key: String) -> Option<Vec<String>> {
    Some(vec![
        row.user_id.clone()?,
        row.date.clone(),
        api_key,
        row.model.clone()?,
        row.custom_llm_provider.clone()?,
        row.mcp_namespaced_tool_name.clone()?,
        row.endpoint.clone()?,
    ])
}

fn check_timestamps(row: &FinOpsUsageRow, row_index: usize) -> Result<(), FinOpsFixtureError> {
    if !is_timestamp(&row.created_at) {
        return Err(FinOpsFixtureError::InvalidTimestamp {
            row: row_index,
            field: "created_at",
        });
    }
    if !is_timestamp(&row.updated_at) {
        return Err(FinOpsFixtureError::InvalidTimestamp {
            row: row_index,
            field: "updated_at",
        });
    }
    Ok(())
}

fn is_timestamp(text: &str) -> bool {
    // the insert casts text to timestamp(3), which drops any non-UTC offset without an error
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
        || NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").is_ok()
        || DateTime::parse_from_rfc3339(text)
            .is_ok_and(|moment| moment.offset().local_minus_utc() == 0_i32)
}

fn config_names(rows: &[ConfigRow]) -> Result<(), FinOpsFixtureError> {
    let mut names = BTreeSet::new();
    for (row, config) in rows.iter().enumerate() {
        if !names.insert(config.name) {
            return Err(FinOpsFixtureError::DuplicateConfigName {
                row,
                name: config.name,
            });
        }
    }
    Ok(())
}
