#![expect(
    clippy::float_arithmetic,
    reason = "LiteLLM defines guardrail timing and risk scoring with floating point arithmetic"
)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailStatus {
    Success,
    GuardrailIntervened,
    GuardrailFailedToRespond,
}

pub fn compute_risk_score(
    detections: &[ContentFilterDetection],
    masked_entity_count: &BTreeMap<String, usize>,
    patterns_checked: usize,
    status: GuardrailStatus,
) -> f64 {
    if status == GuardrailStatus::GuardrailIntervened {
        return 10.0;
    }
    let total_masked = masked_entity_count
        .values()
        .copied()
        .fold(0_usize, usize::saturating_add);
    let masked_as_f64 = u32::try_from(total_masked).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let patterns_as_f64 =
        u32::try_from(patterns_checked).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let match_ratio = if patterns_checked == 0 {
        0.0_f64
    } else {
        masked_as_f64 / patterns_as_f64
    };
    let ratio_score = match_ratio * 7.0_f64;
    let detection_count = u32::try_from(detections.len().min(5)).map_or(5.0_f64, f64::from);
    let detection_score = detection_count * 0.6_f64;
    let mut score = ratio_score + detection_score;
    if total_masked > 0 {
        score = score.max(2.0);
    }
    (score.min(10.0) * 10.0).round() / 10.0
}

#[must_use]
pub fn elapsed_seconds(end_time: f64, start_time: f64) -> f64 {
    if end_time >= start_time {
        end_time - start_time
    } else {
        0.0_f64
    }
}
