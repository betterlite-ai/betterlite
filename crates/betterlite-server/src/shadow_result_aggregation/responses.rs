use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

use super::error::ShadowResultsError;
use super::legs::LegRow;
use super::results::JobResults;
use super::targets::{TargetKey, target_type_name};
use crate::shadow_eval_models::job::ShadowJob;
use crate::shadow_eval_models::leg::ShadowLeg;

// _AttemptCountRow: one leg's attempts and recorded eval spend, judged and errored alike
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegAttemptCount {
    pub job_id: String,
    pub attempt_count: i64,
    pub spend: Decimal,
}

pub type LegCounts = BTreeMap<String, LegAttemptCount>;

// _AttemptTotalsRow: judged turns, errored attempts and judge spend over every attempt
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AttemptTotals {
    pub judged_count: i64,
    pub error_count: i64,
    pub judge_spend: Decimal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ErrorAttempt {
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

// the newest error across the job's legs; a per-leg read leaves one candidate per leg
#[must_use]
pub fn latest_error(attempts: &[ErrorAttempt]) -> Option<String> {
    let mut newest: Option<&ErrorAttempt> = None;
    for attempt in attempts {
        if newest.is_none_or(|current| attempt.created_at > current.created_at) {
            newest = Some(attempt);
        }
    }
    newest?.error.clone()
}

// _group_response: the one constructor of a job response, its config read off the first leg
pub fn group_response(
    group_id: &str,
    legs: &[LegRow],
    counts: &LegCounts,
) -> Result<ShadowJob, ShadowResultsError> {
    let first = legs.first().ok_or(ShadowResultsError::NoLegs)?;
    let mut ordered: Vec<&LegRow> = legs.iter().collect();
    ordered.sort_by(|left, right| {
        target_type_name(left.target_type)
            .cmp(target_type_name(right.target_type))
            .then_with(|| left.target_id.cmp(&right.target_id))
    });
    let targets = ordered.into_iter().map(|leg| target(leg, counts)).collect();
    Ok(ShadowJob {
        job_id: group_id.to_owned(),
        targets,
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

// get_shadow_eval_job: the totals, the latest error and the stratified reads land on the labeled response
pub fn detail_response(
    job: &ShadowJob,
    totals: AttemptTotals,
    last_error: Option<String>,
    results: &JobResults,
) -> Result<ShadowJob, ShadowResultsError> {
    let targets = job
        .targets
        .iter()
        .map(|target| ShadowLeg {
            verdicts: results
                .verdicts
                .get(&TargetKey {
                    target_type: target.target_type,
                    target_id: target.target_id.clone(),
                })
                .cloned(),
            ..target.clone()
        })
        .collect();
    Ok(ShadowJob {
        targets,
        judged_count: Some(totals.judged_count),
        error_count: Some(totals.error_count),
        judge_spend: Some(totals.judge_spend.round_dp(6)),
        last_error,
        results: results.results.clone(),
        ..job.clone()
    })
}

// one target entry; a leg with no counted attempt reads as zero attempts and zero spend
fn target(leg: &LegRow, counts: &LegCounts) -> ShadowLeg {
    let count = counts.get(&leg.id);
    ShadowLeg {
        target_type: leg.target_type,
        target_id: leg.target_id.clone(),
        max_turns: leg.max_turns,
        max_budget: leg.max_budget,
        stopped_at: leg.stopped_at,
        attempt_count: Some(count.map_or(0, |found| found.attempt_count)),
        spend: Some(count.map_or(Decimal::ZERO, |found| found.spend.round_dp(6))),
        verdicts: None,
        target_alias: None,
        key_name: None,
    }
}
