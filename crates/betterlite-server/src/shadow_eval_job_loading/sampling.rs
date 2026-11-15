use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::admission::{canonical_group, direction_admits, sample_hits, scope_admits};
use super::jobs::ActiveShadowEvalJob;

// model_group is the request's group as the payload spells it; the alias map resolves it here
#[derive(Clone, Copy, Debug)]
pub struct RequestAdmission<'request> {
    pub routed_by: Option<&'request str>,
    pub model_group: &'request str,
    pub request_id: &'request str,
}

#[derive(Debug)]
pub struct SampledJobs<'job> {
    pub eligible: Vec<&'job ActiveShadowEvalJob>,
    pub not_sampled: Vec<&'job str>,
}

// an expired or exhausted job is not this request's traffic; a dice-roll miss is counted
#[must_use]
pub fn sampled_jobs<'job>(
    jobs: &[&'job ActiveShadowEvalJob],
    request: &RequestAdmission<'_>,
    starts: &BTreeMap<String, i64>,
    aliases: &BTreeMap<String, String>,
    now: DateTime<Utc>,
) -> SampledJobs<'job> {
    let mut eligible = Vec::new();
    let mut not_sampled = Vec::new();
    let canonical = canonical_group(aliases, request.model_group);
    for job in jobs {
        if now >= job.ends_at {
            continue;
        }
        let started = starts.get(&job.id).copied().unwrap_or(0);
        if job.attempts.saturating_add(started) >= job.max_turns {
            continue;
        }
        if job.max_budget.is_some_and(|budget| job.spend >= budget) {
            continue;
        }
        if !direction_admits(job, request.routed_by) {
            continue;
        }
        if !scope_admits(job, aliases, canonical) {
            continue;
        }
        if sample_hits(request.request_id, &job.id, job.shadow_percentage) {
            eligible.push(*job);
        } else {
            not_sampled.push(job.id.as_str());
        }
    }
    SampledJobs {
        eligible,
        not_sampled,
    }
}
