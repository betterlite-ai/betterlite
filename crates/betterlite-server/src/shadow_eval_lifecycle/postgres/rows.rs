use betterlite_db::postgres_store::errors::backend;
use chrono::NaiveDateTime;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::PostgresLifecycleError;
use super::sql::LEG_COLUMNS;
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_lifecycle::store::{
    ClaimedTarget, LegAttemptStats, LegRow, NewLeg, TargetKeyRow, TargetTeamRow, TargetUserRow,
    parse_target_type,
};

pub(super) fn leg_parameters<'legs>(
    legs: &'legs [NewLeg],
    created: &'legs [NaiveDateTime],
    ends: &'legs [NaiveDateTime],
    kinds: &'legs [&'legs str],
    headings: &'legs [&'legs str],
) -> Vec<&'legs (dyn ToSql + Sync)> {
    let mut parameters: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(legs.len() * LEG_COLUMNS);
    for (index, leg) in legs.iter().enumerate() {
        parameters.push(&leg.id);
        parameters.push(&leg.group_id);
        parameters.push(&kinds[index]);
        parameters.push(&leg.target_id);
        parameters.push(&leg.router_name);
        parameters.push(&leg.router_names);
        parameters.push(&leg.models);
        parameters.push(&headings[index]);
        parameters.push(&leg.baseline_model);
        parameters.push(&leg.judge_model);
        parameters.push(&leg.shadow_percentage);
        parameters.push(&leg.max_turns);
        parameters.push(&leg.max_budget);
        parameters.push(&leg.created_by);
        parameters.push(&created[index]);
        parameters.push(&ends[index]);
    }
    parameters
}

pub(super) fn key_row(row: &Row) -> Result<TargetKeyRow, PostgresLifecycleError> {
    Ok(TargetKeyRow {
        token: row.try_get("token").map_err(backend)?,
        key_alias: row.try_get("key_alias").map_err(backend)?,
        key_name: row.try_get("key_name").map_err(backend)?,
        team_id: row.try_get("team_id").map_err(backend)?,
    })
}

pub(super) fn team_row(row: &Row) -> Result<TargetTeamRow, PostgresLifecycleError> {
    Ok(TargetTeamRow {
        team_id: row.try_get("team_id").map_err(backend)?,
        team_alias: row.try_get("team_alias").map_err(backend)?,
    })
}

pub(super) fn user_row(row: &Row) -> Result<TargetUserRow, PostgresLifecycleError> {
    Ok(TargetUserRow {
        user_id: row.try_get("user_id").map_err(backend)?,
        user_email: row.try_get("user_email").map_err(backend)?,
    })
}

pub(super) fn claimed_row(row: &Row) -> Result<ClaimedTarget, PostgresLifecycleError> {
    let kind: String = row.try_get("target_type").map_err(backend)?;
    let group_id: String = row.try_get("group_id").map_err(backend)?;
    let target_id: String = row.try_get("target_id").map_err(backend)?;
    Ok(ClaimedTarget {
        group_id,
        target_type: parse_target_type(&kind)?,
        target_id,
    })
}

pub(super) fn leg_row(row: &Row) -> Result<LegRow, PostgresLifecycleError> {
    let kind: String = row.try_get("target_type").map_err(backend)?;
    let heading: String = row.try_get("direction").map_err(backend)?;
    let created_at: NaiveDateTime = row.try_get("created_at").map_err(backend)?;
    let ends_at: NaiveDateTime = row.try_get("ends_at").map_err(backend)?;
    let stopped_at: Option<NaiveDateTime> = row.try_get("stopped_at").map_err(backend)?;
    Ok(LegRow {
        id: row.try_get("id").map_err(backend)?,
        group_id: row.try_get("group_id").map_err(backend)?,
        target_type: parse_target_type(&kind)?,
        target_id: row.try_get("target_id").map_err(backend)?,
        router_name: row.try_get("router_name").map_err(backend)?,
        router_names: row.try_get("router_names").map_err(backend)?,
        models: row.try_get("models").map_err(backend)?,
        direction: ShadowEvalDirection::parse(&heading)?,
        baseline_model: row.try_get("baseline_model").map_err(backend)?,
        judge_model: row.try_get("judge_model").map_err(backend)?,
        shadow_percentage: row.try_get("shadow_percentage").map_err(backend)?,
        max_turns: row.try_get("max_turns").map_err(backend)?,
        max_budget: row.try_get("max_budget").map_err(backend)?,
        created_at: created_at.and_utc(),
        ends_at: ends_at.and_utc(),
        stopped_at: stopped_at.map(NaiveDateTime::and_utc),
        stopped_by: row.try_get("stopped_by").map_err(backend)?,
    })
}

pub(super) fn attempt_row(row: &Row) -> Result<LegAttemptStats, PostgresLifecycleError> {
    Ok(LegAttemptStats {
        job_id: row.try_get("job_id").map_err(backend)?,
        attempt_count: row.try_get("attempt_count").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}
