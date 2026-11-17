#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::complexity_scoring::config::TierBoundaries;
use crate::complexity_scoring::dimensions::DimensionScore;
use crate::complexity_scoring::tier::ComplexityTier;

pub const NEGATIVE: f64 = -1.0_f64;
pub const NONE: f64 = 0.0_f64;
pub const HALF: f64 = 0.5_f64;
pub const LOW: f64 = 0.7_f64;
pub const FULL: f64 = 1.0_f64;

// the sum is accumulated in the pinned order: the seven built-in dimensions, then the custom ones
#[must_use]
pub fn weighted_score(
    built_in: &[DimensionScore],
    weights: &BTreeMap<String, f64>,
    custom: &[(DimensionScore, f64)],
) -> f64 {
    let built_in_total: f64 = built_in
        .iter()
        .map(|dimension| {
            let weight = weights
                .get(dimension.dimension.name())
                .copied()
                .unwrap_or(NONE);
            dimension.score * weight
        })
        .sum();
    let custom_total: f64 = custom
        .iter()
        .map(|(dimension, weight)| dimension.score * weight)
        .sum();
    built_in_total + custom_total
}

#[must_use]
pub fn ratio(hits: usize, saturation: usize) -> f64 {
    let hits = f64::from(u32::try_from(hits).unwrap_or(u32::MAX));
    let saturation = f64::from(u32::try_from(saturation).unwrap_or(u32::MAX));
    hits / saturation
}

// a score exactly on a boundary belongs to the tier above it
#[must_use]
pub const fn tier_for(score: f64, boundaries: TierBoundaries) -> ComplexityTier {
    if score < boundaries.simple_medium {
        ComplexityTier::Simple
    } else if score < boundaries.medium_complex {
        ComplexityTier::Medium
    } else if score < boundaries.complex_reasoning {
        ComplexityTier::Complex
    } else {
        ComplexityTier::Reasoning
    }
}

#[must_use]
pub fn within_boundary_margin(score: f64, boundaries: TierBoundaries, margin: f64) -> bool {
    let boundaries = [
        boundaries.simple_medium,
        boundaries.medium_complex,
        boundaries.complex_reasoning,
    ];
    boundaries
        .iter()
        .any(|boundary| (score - boundary).abs() <= margin)
}

// Python takes int(window * buffer), truncating toward zero; a window past u32::MAX is clamped to it, so a request above its buffered capacity is refused
#[must_use]
pub fn window_holds(window: Option<u64>, needed: Option<u64>, buffer: f64) -> bool {
    let (Some(window), Some(needed)) = (window, needed) else {
        return true;
    };
    let window = f64::from(u32::try_from(window).unwrap_or(u32::MAX));
    Decimal::from_f64((window * buffer).trunc())
        .is_some_and(|capacity| Decimal::from(needed) <= capacity)
}
