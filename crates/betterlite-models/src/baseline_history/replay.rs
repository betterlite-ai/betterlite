use std::cmp::Ordering;

use crate::baseline_history::error::BaselineHistoryError;
use crate::baseline_history::estimate::{BaselineEstimate, estimate_for};
use crate::baseline_history::matching::is_plan_valid;
use crate::baseline_history::model::{BaselineHistory, BaselineObservation, CacheEntry, Outcome};
use crate::baseline_history::usage::is_usage_complete;
use crate::baseline_history::writes::{compact_entries, writes_for};

pub const MAX_CACHE_ENTRIES: usize = 1024;

// The storage owner groups and orders observations before calling this function.
pub fn advance_baseline_history(
    history: &BaselineHistory,
    simultaneous: &[BaselineObservation],
) -> Result<(BaselineHistory, Vec<BaselineEstimate>), BaselineHistoryError> {
    let Some(started) = simultaneous.first().map(|item| item.started_at) else {
        return Ok((history.clone(), Vec::new()));
    };
    let valid_order = started.is_finite()
        && simultaneous.iter().all(|item| {
            item.started_at.total_cmp(&started) == Ordering::Equal && item.available_at >= started
        })
        && history.last_at.is_none_or(|last_at| started > last_at);
    if !valid_order {
        let estimates = simultaneous
            .iter()
            .map(|item| BaselineEstimate {
                request_id: item.request_id.clone(),
                reason: "invalid_observation_order".to_owned(),
                provenance: None,
                usage: None,
            })
            .collect();
        return Ok((history.clone(), estimates));
    }
    advance_valid_group(history, simultaneous, started)
}

fn advance_valid_group(
    history: &BaselineHistory,
    simultaneous: &[BaselineObservation],
    started: f64,
) -> Result<(BaselineHistory, Vec<BaselineEstimate>), BaselineHistoryError> {
    let first = history.first_at.unwrap_or(started);
    let uncertain_before = history.uncertain_before.max(first);
    let relevant: Vec<&BaselineObservation> = simultaneous
        .iter()
        .filter(|item| item.outcome != Outcome::ResponseCache)
        .collect();
    let equivalent = history.equivalent && relevant.iter().all(|item| item.baseline_equivalent);
    let before = BaselineHistory {
        first_at: Some(first),
        last_at: history.last_at,
        equivalent,
        uncertain_before,
        entries: history.entries.clone(),
        blocked_until: history.blocked_until,
    };
    let estimates = simultaneous
        .iter()
        .map(|item| estimate_for(&before, item, equivalent))
        .collect::<Result<Vec<_>, _>>()?;
    let (invalidated, blocked) = invalidation_state(history, &relevant);
    let entries = next_entries(history, &before, &relevant, started, invalidated)?;
    let overflow = entries.len() > MAX_CACHE_ENTRIES;
    let next = BaselineHistory {
        first_at: Some(first),
        last_at: Some(started),
        equivalent,
        uncertain_before: if invalidated || overflow {
            started.max(blocked)
        } else {
            uncertain_before
        },
        entries: if overflow { Vec::new() } else { entries },
        blocked_until: blocked,
    };
    Ok((next, estimates))
}

fn invalidation_state(history: &BaselineHistory, relevant: &[&BaselineObservation]) -> (bool, f64) {
    let invalidated = relevant.iter().any(|item| {
        item.outcome != Outcome::Complete
            || !is_usage_complete(item.usage.as_ref())
            || !is_plan_valid(item.plan.as_ref())
    });
    let blocked = if invalidated {
        relevant
            .iter()
            .map(|item| item.available_at)
            .fold(history.blocked_until, f64::max)
    } else {
        history.blocked_until
    };
    (invalidated, blocked)
}

fn next_entries(
    history: &BaselineHistory,
    before: &BaselineHistory,
    relevant: &[&BaselineObservation],
    started: f64,
    invalidated: bool,
) -> Result<Vec<CacheEntry>, BaselineHistoryError> {
    if invalidated {
        return Ok(Vec::new());
    }
    let mut combined = history.entries.clone();
    for item in relevant {
        combined.extend(writes_for(before, item)?);
    }
    Ok(compact_entries(&combined, started))
}

mod tests;
