use std::ops::Sub;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    Healthy,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageTrend {
    Up,
    Down,
    Stable,
}

// the thresholds are LiteLLM's: a rate exactly at 5 or 15 is still the kinder answer
#[must_use]
pub const fn status_from_fail_rate(fail_rate: f64) -> UsageStatus {
    if fail_rate > 15.0 {
        UsageStatus::Critical
    } else if fail_rate > 5.0 {
        UsageStatus::Warning
    } else {
        UsageStatus::Healthy
    }
}

// a rate that moves less than half a percentage point is steady; no previous rate means steady
#[must_use]
pub fn trend_from_comparison(current_fail: f64, previous_fail: f64) -> UsageTrend {
    if previous_fail <= 0.0_f64 {
        return UsageTrend::Stable;
    }
    let diff = Sub::sub(current_fail, previous_fail);
    if diff > 0.5 {
        UsageTrend::Up
    } else if diff < -0.5 {
        UsageTrend::Down
    } else {
        UsageTrend::Stable
    }
}

mod tests;
