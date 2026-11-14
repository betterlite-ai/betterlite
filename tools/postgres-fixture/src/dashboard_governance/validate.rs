use std::collections::BTreeSet;

use super::DashboardGovernanceError;
use super::DashboardGovernanceSeed;
use crate::dashboard::DashboardSeed;
use crate::seed::FixtureSeed;

mod guardrail_indexes;
mod log;
mod metrics;
mod tool_indexes;
mod tools;
mod usage_units;

pub(super) const MAX_ROWS: usize = 128;
pub(super) const IDENTITY_BYTES: usize = 128;
pub(super) const TEXT_BYTES: usize = 4096;

pub(super) fn rows(
    governance: &DashboardGovernanceSeed,
    fixture: &FixtureSeed,
    dashboard_seed: &DashboardSeed,
) -> Result<(), DashboardGovernanceError> {
    check_row_limits(governance)?;
    let teams = fixture
        .teams
        .iter()
        .map(|team| team.team_id.as_str())
        .collect::<BTreeSet<_>>();
    let logs = dashboard_seed
        .logs
        .iter()
        .map(|log| log.request_id.as_str())
        .collect::<BTreeSet<_>>();
    metrics::validate(&governance.metrics)?;
    usage_units::validate(&governance.usage_units, fixture, &teams)?;
    guardrail_indexes::validate(&governance.guardrail_index, &logs)?;
    tool_indexes::validate(&governance.tool_index, &logs)?;
    tools::validate(&governance.tools, fixture, &teams)
}

fn check_row_limits(governance: &DashboardGovernanceSeed) -> Result<(), DashboardGovernanceError> {
    for (list, count) in [
        ("metrics", governance.metrics.len()),
        ("usage_units", governance.usage_units.len()),
        ("guardrail_index", governance.guardrail_index.len()),
        ("tool_index", governance.tool_index.len()),
        ("tools", governance.tools.len()),
    ] {
        if count > MAX_ROWS {
            return Err(DashboardGovernanceError::TooManyRows {
                list,
                count,
                limit: MAX_ROWS,
            });
        }
    }
    Ok(())
}

pub(super) const fn check_text_with_limit(
    list: &'static str,
    row: usize,
    field: &'static str,
    text: &str,
    limit: usize,
) -> Result<(), DashboardGovernanceError> {
    if text.len() > limit {
        return Err(DashboardGovernanceError::TextTooLong {
            list,
            row,
            field,
            bytes: text.len(),
            limit,
        });
    }
    Ok(())
}

pub(super) const fn check_counter(
    list: &'static str,
    row: usize,
    field: &'static str,
    value: i64,
) -> Result<(), DashboardGovernanceError> {
    if value.is_negative() {
        return Err(DashboardGovernanceError::NegativeCounter {
            list,
            row,
            field,
            value,
        });
    }
    Ok(())
}

pub(super) const fn check_key(
    list: &'static str,
    row: usize,
    key_index: Option<usize>,
    keys: usize,
) -> Result<(), DashboardGovernanceError> {
    if let Some(index) = key_index
        && index >= keys
    {
        return Err(DashboardGovernanceError::UnknownKey {
            list,
            row,
            key_index: index,
            keys,
        });
    }
    Ok(())
}

pub(super) fn check_team(
    list: &'static str,
    row: usize,
    team_id: &str,
    teams: &BTreeSet<&str>,
) -> Result<(), DashboardGovernanceError> {
    // LiteLLM stores an empty team_id when the request had no team
    if team_id.is_empty() || teams.contains(team_id) {
        return Ok(());
    }
    Err(DashboardGovernanceError::MissingTeam {
        list,
        row,
        team_id: team_id.to_owned(),
    })
}
