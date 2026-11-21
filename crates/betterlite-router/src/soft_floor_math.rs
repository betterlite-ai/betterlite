#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

use rust_decimal::prelude::ToPrimitive as _;

use crate::adaptive_classification::config::AdaptiveRouterWeights;
use crate::bandit_math::error::BanditMathError;

// L3119: quality_weight * quality_sample + cost_weight * cost_score - penalty_weight * distance
pub fn placement_score(
    weights: AdaptiveRouterWeights,
    quality_sample: f64,
    cost_score: f64,
    tier_distance_penalty: f64,
    tier_distance: usize,
) -> Result<f64, BanditMathError> {
    if !quality_sample.is_finite() {
        return Err(BanditMathError::NonFinite {
            field: "quality sample",
        });
    }
    if !cost_score.is_finite() {
        return Err(BanditMathError::NonFinite {
            field: "cost score",
        });
    }
    if !weights.quality.is_finite() || !weights.economy.is_finite() {
        return Err(BanditMathError::NonFinite {
            field: "score weight",
        });
    }
    if !tier_distance_penalty.is_finite() {
        return Err(BanditMathError::NonFinite {
            field: "tier distance penalty",
        });
    }
    let Some(distance) = tier_distance.to_f64() else {
        return Err(BanditMathError::NonFinite {
            field: "tier distance",
        });
    };
    let quality_term = weights.quality * quality_sample;
    let cost_term = weights.economy * cost_score;
    let distance_term = tier_distance_penalty * distance;
    let score = quality_term + cost_term - distance_term;
    if !score.is_finite() {
        return Err(BanditMathError::NonFinite {
            field: "selection score",
        });
    }
    Ok(score)
}
