use serde::Serialize;

use crate::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use crate::baseline_history::error::BaselineHistoryError;
use crate::baseline_history::model::{BaselineHistory, BaselineObservation, CacheEntry, Outcome};
use crate::baseline_history::time::{checked_add_timestamp, ttl_seconds_as_f64};
use crate::baseline_history::usage::{Usage, is_usage_complete, usage_with_cache};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    ObservedIdentical,
    Modeled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BaselineEstimate {
    pub request_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

const fn outcome_reason(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Complete => "complete",
        Outcome::Uncertain => "uncertain",
        Outcome::ResponseCache => "response_cache",
    }
}

pub(crate) fn is_plan_valid(plan: Option<&CountedPromptCachePlan>) -> bool {
    let Some(candidate) = plan else { return false };
    if candidate.total_tokens < 0 || candidate.breakpoints.len() > 4 {
        return false;
    }
    candidate.breakpoints.iter().all(|marker| {
        !marker.fingerprint.is_empty()
            && !marker.content_fingerprint.is_empty()
            && marker.lookback_fingerprints.contains(&marker.fingerprint)
            && marker
                .lookback_content_fingerprints
                .contains(&marker.content_fingerprint)
            && matches!(marker.ttl_seconds, 300 | 3600)
            && (0..=candidate.total_tokens).contains(&marker.prefix_tokens)
    }) && candidate.breakpoints.windows(2).all(|pair| {
        let [left, right] = pair else { return true };
        left.prefix_tokens <= right.prefix_tokens && left.ttl_seconds >= right.ttl_seconds
    })
}

pub(crate) fn markers_for(
    plan: &CountedPromptCachePlan,
    minimum_cache_tokens: i64,
) -> Vec<&CountedBreakpoint> {
    let mut markers = Vec::new();
    for marker in &plan.breakpoints {
        if marker.prefix_tokens >= minimum_cache_tokens {
            markers.push(marker);
        }
    }
    markers
}

pub(crate) fn entry_matches(
    entry: &CacheEntry,
    markers: &[&CountedBreakpoint],
    started: f64,
) -> bool {
    if entry.available_at > started || started >= entry.expires_at {
        return false;
    }
    markers.iter().any(|marker| {
        marker.lookback_fingerprints.contains(&entry.fingerprint)
            && entry.tokens <= marker.prefix_tokens
            && entry.ttl_seconds == marker.ttl_seconds
    })
}

pub(crate) fn entry_ambiguous(
    entry: &CacheEntry,
    markers: &[&CountedBreakpoint],
    started: f64,
) -> bool {
    if entry.available_at > started || started >= entry.expires_at {
        return false;
    }
    markers.iter().any(|marker| {
        marker
            .lookback_content_fingerprints
            .contains(&entry.content_fingerprint)
            && (entry.uncertain || entry.ttl_seconds != marker.ttl_seconds)
    })
}

struct EstimateContext<'ctx> {
    history: &'ctx BaselineHistory,
    observation: &'ctx BaselineObservation,
    usage: &'ctx Usage,
    plan: &'ctx CountedPromptCachePlan,
    markers: Vec<&'ctx CountedBreakpoint>,
}

pub(crate) fn estimate_for(
    history: &BaselineHistory,
    observation: &BaselineObservation,
    equivalent: bool,
) -> Result<BaselineEstimate, BaselineHistoryError> {
    let Some(usage) = complete_usage_for(observation) else {
        return Ok(incomplete_estimate(observation));
    };
    if equivalent && observation.baseline_equivalent {
        return Ok(BaselineEstimate {
            request_id: observation.request_id.clone(),
            reason: "identical_baseline_path".to_owned(),
            provenance: Some(Provenance::ObservedIdentical),
            usage: Some(usage.clone()),
        });
    }
    if observation.started_at < history.blocked_until {
        return Ok(BaselineEstimate {
            request_id: observation.request_id.clone(),
            reason: "concurrent_uncertainty".to_owned(),
            provenance: None,
            usage: None,
        });
    }
    let Some(plan) = observation
        .plan
        .as_ref()
        .filter(|plan| is_plan_valid(Some(plan)))
    else {
        let reason = observation
            .reason
            .clone()
            .unwrap_or_else(|| "unsupported_cache_plan".to_owned());
        return Ok(BaselineEstimate {
            request_id: observation.request_id.clone(),
            reason,
            provenance: None,
            usage: None,
        });
    };
    plan_estimate(history, observation, usage, plan)
}

fn complete_usage_for(observation: &BaselineObservation) -> Option<&Usage> {
    if observation.outcome != Outcome::Complete {
        return None;
    }
    observation
        .usage
        .as_ref()
        .filter(|usage| is_usage_complete(Some(usage)))
}

fn incomplete_estimate(observation: &BaselineObservation) -> BaselineEstimate {
    let reason = observation
        .reason
        .clone()
        .unwrap_or_else(|| outcome_reason(observation.outcome).to_owned());
    BaselineEstimate {
        request_id: observation.request_id.clone(),
        reason,
        provenance: None,
        usage: None,
    }
}

fn plan_estimate(
    history: &BaselineHistory,
    observation: &BaselineObservation,
    usage: &Usage,
    plan: &CountedPromptCachePlan,
) -> Result<BaselineEstimate, BaselineHistoryError> {
    let markers = markers_for(plan, observation.minimum_cache_tokens);
    if history
        .entries
        .iter()
        .any(|entry| entry_ambiguous(entry, &markers, observation.started_at))
    {
        return Ok(BaselineEstimate {
            request_id: observation.request_id.clone(),
            reason: "cache_ttl_changed".to_owned(),
            provenance: None,
            usage: None,
        });
    }
    let ctx = EstimateContext {
        history,
        observation,
        usage,
        plan,
        markers,
    };
    let read = ctx
        .history
        .entries
        .iter()
        .filter(|entry| {
            !entry.uncertain && entry_matches(entry, &ctx.markers, ctx.observation.started_at)
        })
        .map(|entry| entry.tokens)
        .max()
        .unwrap_or(0);
    let end = ctx.markers.last().map_or(0, |marker| marker.prefix_tokens);
    if is_history_unavailable(&ctx, read, end)? {
        return Ok(BaselineEstimate {
            request_id: ctx.observation.request_id.clone(),
            reason: "history_unavailable".to_owned(),
            provenance: None,
            usage: None,
        });
    }
    cache_hit_estimate(&ctx, read, end)
}

fn is_history_unavailable(
    ctx: &EstimateContext<'_>,
    read: i64,
    end: i64,
) -> Result<bool, BaselineHistoryError> {
    if read >= end {
        return Ok(false);
    }
    let max_ttl = ctx
        .markers
        .iter()
        .map(|marker| marker.ttl_seconds)
        .max()
        .unwrap_or(0);
    let ttl = ttl_seconds_as_f64(max_ttl)?;
    let bound = checked_add_timestamp(ctx.history.uncertain_before, ttl)?;
    Ok(ctx.observation.started_at < bound)
}

fn cache_hit_estimate(
    ctx: &EstimateContext<'_>,
    read: i64,
    end: i64,
) -> Result<BaselineEstimate, BaselineHistoryError> {
    let one_hour = ctx
        .markers
        .iter()
        .filter(|marker| marker.ttl_seconds == 3600 && marker.prefix_tokens > read)
        .map(|marker| marker.prefix_tokens)
        .max()
        .unwrap_or(read);
    let expired = ctx.history.entries.iter().any(|entry| {
        entry.expires_at <= ctx.observation.started_at
            && ctx
                .markers
                .iter()
                .any(|marker| marker.lookback_fingerprints.contains(&entry.fingerprint))
    });
    let reason = cache_prefix_reason(read, expired, ctx.markers.len(), ctx.plan.breakpoints.len());
    let write_short_ttl = end
        .checked_sub(one_hour)
        .ok_or(BaselineHistoryError::CounterOverflow)?;
    let write_long_ttl = one_hour
        .checked_sub(read)
        .ok_or(BaselineHistoryError::CounterOverflow)?;
    let usage = usage_with_cache(
        ctx.usage,
        ctx.plan.total_tokens,
        read,
        write_short_ttl,
        write_long_ttl,
    )?;
    Ok(BaselineEstimate {
        request_id: ctx.observation.request_id.clone(),
        reason: reason.to_owned(),
        provenance: Some(Provenance::Modeled),
        usage: Some(usage),
    })
}

const fn cache_prefix_reason(
    read: i64,
    expired: bool,
    marker_count: usize,
    breakpoint_count: usize,
) -> &'static str {
    if read > 0 {
        "cache_prefix_available"
    } else if expired {
        "cache_prefix_expired"
    } else if marker_count > 0 {
        "cache_prefix_cold"
    } else if breakpoint_count > 0 {
        "below_cache_minimum"
    } else {
        "no_cache_breakpoints"
    }
}

mod tests;
