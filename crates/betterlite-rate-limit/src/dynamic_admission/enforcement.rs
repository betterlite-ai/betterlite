use crate::atomic_counter::payload::IncrementAmounts;
use crate::limiter_records::RateLimitDescriptor;

mod tests;

// _check_rate_limits' enforced set and the priority pools it only tracks until the model is saturated
pub struct EnforcementPlan {
    pub enforced: Vec<RateLimitDescriptor>,
    pub increments: Vec<IncrementAmounts>,
    pub tracked_only: Vec<RateLimitDescriptor>,
}

// the model-wide descriptor always runs first; the priority descriptors join it at the threshold, and an equal saturation enforces
#[must_use]
pub fn plan_enforcement(
    model_wide: RateLimitDescriptor,
    priority_descriptors: Vec<RateLimitDescriptor>,
    saturation: f64,
    saturation_threshold: f64,
) -> EnforcementPlan {
    let enforce_priority = saturation >= saturation_threshold;
    let mut enforced = Vec::with_capacity(priority_descriptors.len().saturating_add(1));
    enforced.push(model_wide);
    let tracked_only = if enforce_priority {
        enforced.extend(priority_descriptors);
        Vec::new()
    } else {
        priority_descriptors
    };
    let increments = vec![
        IncrementAmounts {
            requests: 1,
            tokens: 0,
        };
        enforced.len()
    ];
    EnforcementPlan {
        enforced,
        increments,
        tracked_only,
    }
}
