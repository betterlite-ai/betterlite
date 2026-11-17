#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

pub mod error;
pub mod sampler;

mod tests;

use crate::adaptive_classification::config::{AdaptiveRouterPreferences, AdaptiveRouterWeights};
use crate::adaptive_classification::constants::{
    BASE_TIER_WEIGHT, COLD_START_MASS, SAMPLE_CAP, STRENGTH_BONUS,
};
use crate::adaptive_classification::request_type::RequestType;

use self::error::BanditMathError;
use self::sampler::QualitySampler;

const PRIOR_MEAN_CEILING: f64 = 0.95;
const EMPTY_CELL_MEAN: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BanditCell {
    pub alpha: f64,
    pub beta: f64,
}

impl BanditCell {
    pub const fn new(alpha: f64, beta: f64) -> Result<Self, BanditMathError> {
        if !alpha.is_finite() {
            return Err(BanditMathError::NonFinite { field: "alpha" });
        }
        if !beta.is_finite() {
            return Err(BanditMathError::NonFinite { field: "beta" });
        }
        if !(alpha + beta).is_finite() {
            return Err(BanditMathError::NonFinite {
                field: "alpha+beta",
            });
        }
        Ok(Self { alpha, beta })
    }

    #[must_use]
    pub const fn mean(self) -> f64 {
        let total = self.alpha + self.beta;
        if total > 0.0 {
            self.alpha / total
        } else {
            EMPTY_CELL_MEAN
        }
    }

    #[must_use]
    pub fn total_samples(self) -> u64 {
        let observed = self.alpha + self.beta - COLD_START_MASS;
        if observed <= 0.0 {
            return 0;
        }
        let text = format!("{:.0}", observed.trunc());
        text.parse().unwrap_or(u64::MAX)
    }

    #[must_use]
    pub const fn quality_mean(self) -> f64 {
        if self.alpha + self.beta > 0.0 {
            self.mean()
        } else {
            0.0
        }
    }
}

pub fn initial_cell(
    preferences: &AdaptiveRouterPreferences,
    request_type: RequestType,
) -> Result<BanditCell, BanditMathError> {
    let Some(weight) = BASE_TIER_WEIGHT
        .iter()
        .find(|(tier, _weight)| *tier == preferences.quality_tier)
        .map(|(_tier, weight)| *weight)
    else {
        return Err(BanditMathError::UnsupportedTier {
            tier: preferences.quality_tier,
        });
    };
    let bonus = if preferences.strengths.contains(&request_type) {
        STRENGTH_BONUS
    } else {
        0.0
    };
    let prior = (weight + bonus).min(PRIOR_MEAN_CEILING);
    BanditCell::new(prior * COLD_START_MASS, (1.0 - prior) * COLD_START_MASS)
}

pub fn apply_delta(
    cell: BanditCell,
    delta_alpha: f64,
    delta_beta: f64,
) -> Result<BanditCell, BanditMathError> {
    let new_alpha = cell.alpha + delta_alpha;
    let new_beta = cell.beta + delta_beta;
    let cap = f64::from(u32::try_from(SAMPLE_CAP).unwrap_or(u32::MAX));
    if new_alpha + new_beta > cap {
        return Ok(cell);
    }
    BanditCell::new(new_alpha, new_beta)
}

pub const fn score(
    quality_sample: f64,
    cost_score: f64,
    weights: AdaptiveRouterWeights,
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
    Ok(weights.quality * quality_sample + weights.economy * cost_score)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate<'model> {
    pub model: &'model str,
    pub cell: BanditCell,
    pub economy_score: f64,
}

pub fn pick_best(
    candidates: &[Candidate<'_>],
    weights: AdaptiveRouterWeights,
    sampler: &mut QualitySampler,
) -> Result<String, BanditMathError> {
    let mut best: Option<(&str, f64)> = None;
    for candidate in candidates {
        let quality = sampler.sample(candidate.cell)?;
        let candidate_score = score(quality, candidate.economy_score, weights)?;
        let improves = best.is_none_or(|(_model, best_score)| candidate_score > best_score);
        if improves {
            best = Some((candidate.model, candidate_score));
        }
    }
    let Some((model, _score)) = best else {
        return Err(BanditMathError::NoModels);
    };
    Ok(model.to_owned())
}
