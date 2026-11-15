use std::collections::BTreeMap;

use num_traits::ToPrimitive as _;
use sha2::{Digest as _, Sha256};

use super::jobs::{ActiveShadowEvalJob, ShadowEvalDirection};

#[must_use]
pub fn direction_admits(job: &ActiveShadowEvalJob, routed_by: Option<&str>) -> bool {
    match job.direction {
        ShadowEvalDirection::Reverse => routed_by == Some(job.router_name.as_str()),
        ShadowEvalDirection::Forward => !job
            .arm_router_names
            .iter()
            .any(|arm| routed_by == Some(arm.as_str())),
    }
}

#[must_use]
pub fn canonical_group<'aliases>(
    aliases: &'aliases BTreeMap<String, String>,
    model_group: &'aliases str,
) -> &'aliases str {
    match aliases.get(model_group) {
        Some(target) if !target.is_empty() => target.as_str(),
        _ => model_group,
    }
}

// model_group arrives canonical: the hook resolves the request's group once, before the gate
#[must_use]
pub fn scope_admits(
    job: &ActiveShadowEvalJob,
    aliases: &BTreeMap<String, String>,
    model_group: &str,
) -> bool {
    if job.models.is_empty() {
        return true;
    }
    job.models
        .iter()
        .any(|name| canonical_group(aliases, name) == model_group)
}

// the same sha256 slice both pods agree on: the leading eight bytes over 2^64, times 100
#[must_use]
pub fn sample_hits(request_id: &str, job_id: &str, percentage: f64) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(job_id.as_bytes());
    hasher.update(b":");
    hasher.update(request_id.as_bytes());
    let digest = hasher.finalize();
    let mut head = [0_u8; 8];
    for (slot, byte) in head.iter_mut().zip(digest.iter()) {
        *slot = *byte;
    }
    let fraction = u64::from_be_bytes(head)
        .to_f64()
        .unwrap_or(0.0)
        .mul_add(2.0_f64.powi(-64), 0.0);
    fraction.mul_add(100.0, 0.0) < percentage
}

#[must_use]
pub fn job_spend_counter_key(job_id: &str) -> String {
    let mut key = String::from("spend:shadow_eval:");
    key.push_str(job_id);
    key
}
