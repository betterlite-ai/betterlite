use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;

use crate::shadow_eval_lifecycle::store::{LegAttemptStats, LegRow, TargetRows};
use crate::shadow_eval_models::job::{ShadowJob, ShadowLeg};
use crate::shadow_eval_models::{ShadowEvalModelError, ShadowEvalTargetType};

// Python sorts the (target_type, target_id) pairs as text, so key rows come before team rows before user rows
const fn target_rank(target_type: ShadowEvalTargetType) -> u8 {
    match target_type {
        ShadowEvalTargetType::Key => 0,
        ShadowEvalTargetType::Team => 1,
        ShadowEvalTargetType::User => 2,
    }
}

// _group_response: the one constructor of a job response from its legs; None stats keeps the start response's nulls
pub fn job_from_legs(
    group_id: &str,
    legs: &[LegRow],
    stats: Option<&[LegAttemptStats]>,
) -> Result<ShadowJob, ShadowEvalModelError> {
    let first = legs.first().ok_or(ShadowEvalModelError::NoTargets)?;
    let by_leg: BTreeMap<&str, &LegAttemptStats> = stats
        .unwrap_or_default()
        .iter()
        .map(|row| (row.job_id.as_str(), row))
        .collect();
    let mut ordered: Vec<&LegRow> = legs.iter().collect();
    ordered.sort_by(|left, right| {
        (target_rank(left.target_type), left.target_id.as_str())
            .cmp(&(target_rank(right.target_type), right.target_id.as_str()))
    });
    Ok(ShadowJob {
        job_id: group_id.to_owned(),
        targets: ordered
            .into_iter()
            .map(|leg| leg_response(leg, stats, &by_leg))
            .collect(),
        router_names: first.arm_router_names(),
        models: first.models.clone(),
        direction: first.direction,
        baseline_model: first.baseline_model.clone(),
        judge_model: first.judge_model.clone(),
        shadow_percentage: first.shadow_percentage,
        created_at: first.created_at,
        ends_at: first.ends_at,
        stopped_by: legs.iter().find_map(|leg| leg.stopped_by.clone()),
        judged_count: None,
        error_count: None,
        judge_spend: None,
        last_error: None,
        results: None,
    })
}

// A counted read fills 0 and 0.0 for a leg with no attempt rows; the start response passes no counts at all
fn leg_response(
    leg: &LegRow,
    stats: Option<&[LegAttemptStats]>,
    by_leg: &BTreeMap<&str, &LegAttemptStats>,
) -> ShadowLeg {
    let counted = stats.map(|_| by_leg.get(leg.id.as_str()).copied());
    ShadowLeg {
        target_type: leg.target_type,
        target_id: leg.target_id.clone(),
        max_turns: leg.max_turns,
        max_budget: leg.max_budget,
        stopped_at: leg.stopped_at,
        attempt_count: counted.map(|row| row.map_or(0, |row| row.attempt_count)),
        spend: counted.map(|row| row.map_or(Decimal::ZERO, |row| row.spend.round_dp(6))),
        verdicts: None,
        target_alias: None,
        key_name: None,
    }
}

// _with_target_labels: each target id resolves to its display label at read time, and a deleted one to nothing
pub fn apply_target_labels(job: &mut ShadowJob, rows: &TargetRows) {
    for target in &mut job.targets {
        let labels = match target.target_type {
            ShadowEvalTargetType::Key => rows
                .keys
                .iter()
                .find(|row| row.token == target.target_id)
                .map(|row| (row.key_alias.clone(), row.key_name.clone())),
            ShadowEvalTargetType::Team => rows
                .teams
                .iter()
                .find(|row| row.team_id == target.target_id)
                .map(|row| (row.team_alias.clone(), None)),
            ShadowEvalTargetType::User => rows
                .users
                .iter()
                .find(|row| row.user_id == target.target_id)
                .map(|row| (row.user_email.clone(), None)),
        };
        let (target_alias, key_name) = labels.unwrap_or_default();
        target.target_alias = target_alias;
        target.key_name = key_name;
    }
}

// _target_ids_of: every distinct id a job scopes for one target kind, sorted for a batched label read
#[must_use]
pub fn target_ids(legs: &[LegRow], target_type: ShadowEvalTargetType) -> Vec<String> {
    legs.iter()
        .filter(|leg| leg.target_type == target_type)
        .map(|leg| leg.target_id.clone())
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}
