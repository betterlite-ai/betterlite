use crate::shadow_eval_lifecycle::store::TargetRows;
use crate::shadow_eval_models::job::ShadowJob;
use crate::shadow_result_aggregation::error::ShadowResultsError;
use crate::shadow_result_aggregation::labels::{LabelRow, target_labels};
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::responses::{LegAttemptCount, LegCounts, group_response};
use crate::shadow_result_aggregation::targets::TargetKey;

// _group_response, except that a start passes no counts and its targets keep the response model's nulls
pub fn job_from_legs(
    group_id: &str,
    legs: &[LegRow],
    stats: Option<&[LegAttemptCount]>,
) -> Result<ShadowJob, ShadowResultsError> {
    let counts: LegCounts = stats
        .unwrap_or_default()
        .iter()
        .map(|row| (row.job_id.clone(), row.clone()))
        .collect();
    let mut job = group_response(group_id, legs, &counts)?;
    if stats.is_none() {
        for target in &mut job.targets {
            target.attempt_count = None;
            target.spend = None;
        }
    }
    Ok(job)
}

// _with_target_labels: each target id resolves to its display label at read time, and a deleted one to nothing
pub fn apply_target_labels(job: &mut ShadowJob, rows: &TargetRows) {
    let (keys, teams, users) = label_rows(rows);
    let labels = target_labels(&keys, &teams, &users);
    for target in &mut job.targets {
        let found = labels
            .get(&TargetKey {
                target_type: target.target_type,
                target_id: target.target_id.clone(),
            })
            .cloned()
            .unwrap_or_default();
        target.target_alias = found.0;
        target.key_name = found.1;
    }
}

// the loaded target rows in the shape _target_labels reads: a key's (alias, masked name), a team's and a user's (label, None)
fn label_rows(rows: &TargetRows) -> (Vec<LabelRow>, Vec<LabelRow>, Vec<LabelRow>) {
    let keys = rows
        .keys
        .iter()
        .map(|row| LabelRow {
            target_id: row.token.clone(),
            target_alias: row.key_alias.clone(),
            key_name: row.key_name.clone(),
        })
        .collect();
    let teams = rows
        .teams
        .iter()
        .map(|row| LabelRow {
            target_id: row.team_id.clone(),
            target_alias: row.team_alias.clone(),
            key_name: None,
        })
        .collect();
    let users = rows
        .users
        .iter()
        .map(|row| LabelRow {
            target_id: row.user_id.clone(),
            target_alias: row.user_email.clone(),
            key_name: None,
        })
        .collect();
    (keys, teams, users)
}
