use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_result_aggregation::error::ShadowResultsError;
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::repository::ShadowResultsRepository;
use crate::shadow_result_aggregation::responses::{AttemptTotals, detail_response, group_response};

use super::error::ShadowResultsRouteError;
use super::query::ShadowQuery;
use super::store::{ShadowLegReads, ShadowTargetFilter};

// list_shadow_eval_jobs: the newest jobs, every leg of each, one counts read and one label read per kind
pub async fn list_shadow_eval_jobs<Store>(
    store: &Store,
    query: &ShadowQuery,
    now: DateTime<Utc>,
) -> Result<Vec<ShadowJobResponse>, ShadowResultsRouteError>
where
    Store: ShadowResultsRepository + ShadowLegReads + ?Sized,
{
    let target = match (&query.target_type, &query.target_id) {
        (Some(target_type), Some(target_id)) => Some(ShadowTargetFilter {
            target_type: *target_type,
            target_id: target_id.clone(),
        }),
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => {
            return Err(ShadowResultsRouteError::FilterPair);
        }
    };
    let legs = store.list_legs(query.limit, target).await?;
    let counts = store.leg_attempt_counts(&legs).await?;
    let mut responses = Vec::new();
    for (group_id, group_legs) in newest_first(legs) {
        responses.push(group_response(&group_id, &group_legs, &counts)?);
    }
    let labeled = store.labeled_responses(&responses).await?;
    labeled
        .into_iter()
        .map(|job| job.response(now).map_err(ShadowResultsRouteError::from))
        .collect()
}

// get_shadow_eval_job: totals, the latest error, each target's counts, labels, then the stratified reads
pub async fn get_shadow_eval_job<Store>(
    store: &Store,
    job_id: &str,
    now: DateTime<Utc>,
) -> Result<ShadowJobResponse, ShadowResultsRouteError>
where
    Store: ShadowResultsRepository + ShadowLegReads + ?Sized,
{
    let legs = store.legs_of_group(job_id).await?;
    if legs.is_empty() {
        return Err(ShadowResultsRouteError::JobNotFound {
            job_id: job_id.to_owned(),
        });
    }
    let leg_ids: Vec<String> = legs.iter().map(|leg| leg.id.clone()).collect();
    let totals = match store.attempt_totals(&leg_ids).await {
        Ok(totals) => totals,
        Err(ShadowResultsError::NoTotalsRow) => AttemptTotals {
            judged_count: 0,
            error_count: 0,
            judge_spend: Decimal::ZERO,
        },
        Err(error) => return Err(ShadowResultsRouteError::from(error)),
    };
    let last_error = store.latest_error(&leg_ids).await?;
    let counts = store.leg_attempt_counts(&legs).await?;
    let job = group_response(job_id, &legs, &counts)?;
    let mut labeled = store.labeled_responses(&[job]).await?;
    let Some(labeled_job) = labeled.pop() else {
        return Err(ShadowResultsRouteError::from(ShadowResultsError::NoLegs));
    };
    let results = store.job_results(&legs).await?;
    let detail = detail_response(&labeled_job, totals, last_error, &results);
    Ok(detail.response(now)?)
}

// jobs by group id, the group whose newest leg is newest first; a tie keeps ascending group id
fn newest_first(legs: Vec<LegRow>) -> Vec<(String, Vec<LegRow>)> {
    let mut by_group: BTreeMap<String, Vec<LegRow>> = BTreeMap::new();
    for leg in legs {
        by_group.entry(leg.group_id.clone()).or_default().push(leg);
    }
    let mut groups: Vec<(String, Vec<LegRow>)> = by_group.into_iter().collect();
    groups.sort_by_key(|(_group_id, group_legs)| {
        Reverse(group_legs.iter().map(|leg| leg.created_at).max())
    });
    groups
}
