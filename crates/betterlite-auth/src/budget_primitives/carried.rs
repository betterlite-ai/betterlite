use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{Datelike as _, NaiveDateTime, Timelike as _};
use serde_json::value::RawValue;
use speedate::{Date, DateTime as SpeedateDateTime, Time};
use std::collections::BTreeMap;

use crate::budget_snapshots::lax_datetime::LaxDateTime;
use crate::budget_snapshots::{
    BudgetSnapshot, BudgetSnapshotError, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use crate::resolved_key::auth::UserAPIKeyAuth;

pub fn carry_team_and_user(
    token: &mut UserAPIKeyAuth,
    team: Option<&LiteLLMTeamTable>,
    user: Option<&LiteLLMUserTable>,
) {
    if let Some(team_row) = team {
        token.extra.team_budget_snapshot = Some(TeamBudgetSnapshot {
            budget_reset_at: team_row.fields.budget_reset_at.and_then(snapshot_datetime),
            max_budget: team_row.fields.base.max_budget,
        });
        token.view.team_model_max_budget = team_row.fields.model_max_budget.0.clone().map(Payload);
    }
    if let Some(user_row) = user {
        token.extra.user_budget_snapshot = Some(UserBudgetSnapshot {
            budget_reset_at: user_row.budget_reset_at.and_then(snapshot_datetime),
            max_budget: user_row.max_budget,
            user_alias: user_row.user_alias.clone(),
        });
    }
}

pub fn carry_organization(token: &mut UserAPIKeyAuth, organization: &OrganizationTable) {
    token
        .view
        .organization_alias
        .clone_from(&organization.organization_alias);
    token.extra.org_budget_snapshot = Some(OrgBudgetSnapshot {
        spend: organization.spend,
        max_budget: organization
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| budget.max_budget),
    });
}

pub fn carried_metadata(
    token: &UserAPIKeyAuth,
) -> Result<BTreeMap<String, Box<RawValue>>, BudgetSnapshotError> {
    // team, then user, then organization; a later snapshot's key wins when two carry the same one
    let snapshots = [
        token
            .extra
            .team_budget_snapshot
            .as_ref()
            .map(BudgetSnapshot::metadata_entries)
            .transpose()?,
        token
            .extra
            .user_budget_snapshot
            .as_ref()
            .map(BudgetSnapshot::metadata_entries)
            .transpose()?,
        token
            .extra
            .org_budget_snapshot
            .as_ref()
            .map(BudgetSnapshot::metadata_entries)
            .transpose()?,
    ];
    let mut merged = BTreeMap::new();
    for entries in snapshots.into_iter().flatten() {
        merged.extend(
            entries
                .as_ref()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    Ok(merged)
}

fn snapshot_datetime(timestamp: Timestamp) -> Option<LaxDateTime> {
    match timestamp {
        Timestamp::Aware(value) => {
            let text = speedate_text(value.naive_local(), Some(value.offset().local_minus_utc()))?;
            Some(LaxDateTime::Offset(text))
        }
        Timestamp::Naive(value) => Some(LaxDateTime::Naive(speedate_text(value, None)?)),
    }
}

fn speedate_text(value: NaiveDateTime, tz_offset: Option<i32>) -> Option<String> {
    let date = Date {
        year: u16::try_from(value.year()).ok()?,
        month: u8::try_from(value.month()).ok()?,
        day: u8::try_from(value.day()).ok()?,
    };
    let time = Time {
        hour: u8::try_from(value.hour()).ok()?,
        minute: u8::try_from(value.minute()).ok()?,
        second: u8::try_from(value.second()).ok()?,
        microsecond: value.and_utc().timestamp_subsec_micros(),
        tz_offset,
    };
    Some(SpeedateDateTime { date, time }.to_string())
}
