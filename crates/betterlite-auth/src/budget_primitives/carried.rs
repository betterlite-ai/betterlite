use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{Datelike as _, FixedOffset, NaiveDateTime, Timelike as _};
use serde_json::value::RawValue;
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
            budget_reset_at: team_row.fields.budget_reset_at.map(snapshot_datetime),
            max_budget: team_row.fields.base.max_budget,
        });
        token.view.team_model_max_budget = team_row.fields.model_max_budget.0.clone().map(Payload);
    }
    if let Some(user_row) = user {
        token.extra.user_budget_snapshot = Some(UserBudgetSnapshot {
            budget_reset_at: user_row.budget_reset_at.map(snapshot_datetime),
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

fn snapshot_datetime(timestamp: Timestamp) -> LaxDateTime {
    match timestamp {
        Timestamp::Aware(value) => LaxDateTime::Offset(pydantic_datetime(
            value.naive_local(),
            Some(*value.offset()),
        )),
        Timestamp::Naive(value) => LaxDateTime::Naive(pydantic_datetime(value, None)),
    }
}

fn pydantic_datetime(value: NaiveDateTime, offset: Option<FixedOffset>) -> String {
    let date = format!(
        "{:04}-{:02}-{:02}",
        value.year(),
        value.month(),
        value.day()
    );
    let clock = format!(
        "{:02}:{:02}:{:02}",
        value.hour(),
        value.minute(),
        value.second()
    );
    let micro = value.and_utc().timestamp_subsec_micros();
    let fraction = if micro == 0 {
        String::new()
    } else {
        format!(".{micro:06}")
    };
    let zone = match offset {
        None => String::new(),
        Some(zone) if zone.local_minus_utc() == 0_i32 => "Z".to_owned(),
        Some(zone) => offset_text(zone),
    };
    format!("{date}T{clock}{fraction}{zone}")
}

fn offset_text(offset: FixedOffset) -> String {
    let total = offset.local_minus_utc();
    let sign = if total < 0_i32 { '-' } else { '+' };
    let magnitude = total.unsigned_abs();
    let hours = magnitude.checked_div(3_600).unwrap_or(0);
    let minutes = magnitude
        .checked_rem(3_600)
        .and_then(|rest| rest.checked_div(60))
        .unwrap_or(0);
    let seconds = magnitude.checked_rem(60).unwrap_or(0);
    if seconds == 0 {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    }
}
