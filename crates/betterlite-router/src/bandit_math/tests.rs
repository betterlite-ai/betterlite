#![cfg(test)]

use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use super::error::BanditMathError;
use super::sampler::{QualitySampler, ScriptedSampler};
use super::{
    BanditCell, Candidate, EMPTY_CELL_MEAN, PRIOR_MEAN_CEILING, apply_delta, initial_cell,
    pick_best, score,
};
use crate::adaptive_classification::config::{AdaptiveRouterPreferences, AdaptiveRouterWeights};
use crate::adaptive_classification::request_type::RequestType;

fn preferences(tier: u8, strengths: &[RequestType]) -> AdaptiveRouterPreferences {
    AdaptiveRouterPreferences {
        quality_tier: tier,
        strengths: strengths.to_vec(),
    }
}

const fn weights(quality: f64, economy: f64) -> AdaptiveRouterWeights {
    AdaptiveRouterWeights { quality, economy }
}

fn cell(alpha: f64, beta: f64) -> BanditCell {
    BanditCell::new(alpha, beta).unwrap()
}

fn assert_cell(bandit_cell: BanditCell, alpha: &str, beta: &str, samples: u64) {
    assert_eq!(format!("{:.6}", bandit_cell.alpha), alpha);
    assert_eq!(format!("{:.6}", bandit_cell.beta), beta);
    assert_eq!(bandit_cell.total_samples(), samples);
}

#[rstest]
#[case::lowest_tier(1, &[], RequestType::General, "3.000000", "7.000000")]
#[case::middle_tier(2, &[], RequestType::Writing, "5.000000", "5.000000")]
#[case::highest_tier(3, &[], RequestType::General, "7.000000", "3.000000")]
#[case::strength_bonus(
    2,
    &[RequestType::CodeGeneration],
    RequestType::CodeGeneration,
    "8.000000",
    "2.000000"
)]
#[case::strength_elsewhere(
    2,
    &[RequestType::Writing],
    RequestType::CodeGeneration,
    "5.000000",
    "5.000000"
)]
#[case::capped_top_strength(
    3,
    &[RequestType::Writing],
    RequestType::Writing,
    "9.500000",
    "0.500000"
)]
fn cold_priors_follow_the_tier_weight_and_strength_bonus(
    #[case] tier: u8,
    #[case] strengths: &[RequestType],
    #[case] request_type: RequestType,
    #[case] alpha: &str,
    #[case] beta: &str,
) {
    let prior = initial_cell(&preferences(tier, strengths), request_type).unwrap();
    assert_cell(prior, alpha, beta, 0);
}

#[rstest]
#[case::tier_zero(0)]
#[case::tier_four(4)]
#[case::tier_above_the_byte_range(255)]
fn an_unsupported_tier_names_the_supported_tiers(#[case] tier: u8) {
    let error = initial_cell(&preferences(tier, &[]), RequestType::General).unwrap_err();
    assert_eq!(error, BanditMathError::UnsupportedTier { tier });
    assert_eq!(
        error.to_string(),
        format!("quality_tier={tier} is not supported; valid tiers are [1, 2, 3]")
    );
}

#[rstest]
#[case::prior_mass(3.0_f64, 7.0_f64, "0.300000", "0.300000", 0)]
#[case::balanced_prior(5.0_f64, 5.0_f64, "0.500000", "0.500000", 0)]
#[case::empty_falls_back(0.0_f64, 0.0_f64, "0.500000", "0.000000", 0)]
#[case::observed_mass(11.0_f64, 9.0_f64, "0.550000", "0.550000", 10)]
#[case::fractional_mass_truncates(10.4_f64, 10.4_f64, "0.500000", "0.500000", 10)]
#[case::even_mass(12.0_f64, 8.0_f64, "0.600000", "0.600000", 10)]
#[case::positive_total_under_the_prior(0.5_f64, 0.0_f64, "1.000000", "1.000000", 0)]
#[case::negative_total_falls_back(-5.0_f64, 4.0_f64, "0.500000", "0.000000", 0)]
fn cells_report_their_mean_mass_and_samples(
    #[case] alpha: f64,
    #[case] beta: f64,
    #[case] mean: &str,
    #[case] quality_mean: &str,
    #[case] samples: u64,
) {
    let measured = cell(alpha, beta);
    assert_eq!(format!("{:.6}", measured.mean()), mean);
    assert_eq!(format!("{:.6}", measured.quality_mean()), quality_mean);
    assert_eq!(measured.total_samples(), samples);
}

#[rstest]
#[case::nan_alpha(f64::NAN, 1.0_f64, "alpha")]
#[case::infinite_beta(1.0_f64, f64::INFINITY, "beta")]
#[case::overflowing_sum(f64::MAX, f64::MAX, "alpha+beta")]
fn a_non_finite_cell_names_the_field(
    #[case] alpha: f64,
    #[case] beta: f64,
    #[case] field: &'static str,
) {
    assert_eq!(
        BanditCell::new(alpha, beta),
        Err(BanditMathError::NonFinite { field })
    );
}

#[rstest]
#[case::under_the_cap((50.0_f64, 50.0_f64), (1.0_f64, 1.0_f64), ("51.000000", "51.000000"), 92)]
#[case::at_the_cap((99.0_f64, 99.0_f64), (1.0_f64, 1.0_f64), ("100.000000", "100.000000"), 190)]
#[case::fractional_at_the_cap((199.5_f64, 0.0_f64), (0.5_f64, 0.0_f64), ("200.000000", "0.000000"), 190)]
#[case::over_the_cap_keeps_the_cell(
    (100.0_f64, 99.0_f64),
    (1.0_f64, 1.0_f64),
    ("100.000000", "99.000000"),
    189
)]
#[case::fractional_over_the_cap_keeps_it(
    (199.5_f64, 0.0_f64),
    (0.5_f64, 0.000_001_f64),
    ("199.500000", "0.000000"),
    189
)]
#[case::negative_delta_shrinks(
    (100.0_f64, 100.0_f64),
    (-50.0_f64, -50.0_f64),
    ("50.000000", "50.000000"),
    90
)]
fn updates_respect_the_sample_cap(
    #[case] cell_values: (f64, f64),
    #[case] deltas: (f64, f64),
    #[case] expected_cell: (&str, &str),
    #[case] expected_samples: u64,
) {
    let (alpha, beta) = cell_values;
    let (delta_alpha, delta_beta) = deltas;
    let (expected_alpha, expected_beta) = expected_cell;
    let updated = apply_delta(cell(alpha, beta), delta_alpha, delta_beta).unwrap();
    assert_cell(updated, expected_alpha, expected_beta, expected_samples);
}

#[test]
fn a_non_finite_delta_is_refused() {
    let error = apply_delta(cell(5.0, 5.0), f64::NAN, 0.0).unwrap_err();
    assert_eq!(error, BanditMathError::NonFinite { field: "alpha" });
}

#[rstest]
#[case::quality_only(0.8_f64, 0.0_f64, 0.7_f64, 0.3_f64, "0.560000")]
#[case::cost_only(0.0_f64, 0.5_f64, 0.7_f64, 0.3_f64, "0.150000")]
#[case::mixed(0.5_f64, 0.5_f64, 0.7_f64, 0.3_f64, "0.500000")]
#[case::all_quality(0.4_f64, 0.9_f64, 1.0_f64, 0.0_f64, "0.400000")]
#[case::all_cost(0.4_f64, 0.9_f64, 0.0_f64, 1.0_f64, "0.900000")]
fn the_score_is_the_weighted_sum(
    #[case] quality_sample: f64,
    #[case] cost_score: f64,
    #[case] quality_weight: f64,
    #[case] cost_weight: f64,
    #[case] expected: &str,
) {
    let value = score(
        quality_sample,
        cost_score,
        weights(quality_weight, cost_weight),
    )
    .unwrap();
    assert_eq!(format!("{value:.6}"), expected);
}

#[test]
fn the_weighted_score_fuses_the_quality_product() {
    let fused = score(0.75, 0.25, weights(0.7, 0.3)).unwrap();
    let quality_term = 0.7_f64 * 0.75_f64;
    let economy_term = 0.3_f64 * 0.25_f64;
    let separate = quality_term + economy_term;
    assert_eq!(fused.to_bits(), 0.6_f64.to_bits());
    assert_ne!(fused.to_bits(), separate.to_bits());
}

#[rstest]
#[case::quality_sample((f64::NAN, 0.5_f64), (0.7_f64, 0.3_f64), "quality sample")]
#[case::cost_score((0.5_f64, f64::INFINITY), (0.7_f64, 0.3_f64), "cost score")]
#[case::quality_weight((0.5_f64, 0.5_f64), (f64::NAN, 0.3_f64), "score weight")]
#[case::cost_weight((0.5_f64, 0.5_f64), (0.7_f64, f64::NEG_INFINITY), "score weight")]
fn a_non_finite_score_names_the_field(
    #[case] sample: (f64, f64),
    #[case] weight: (f64, f64),
    #[case] field: &'static str,
) {
    let (quality_sample, cost_score) = sample;
    let (quality_weight, cost_weight) = weight;
    assert_eq!(
        score(
            quality_sample,
            cost_score,
            weights(quality_weight, cost_weight)
        ),
        Err(BanditMathError::NonFinite { field })
    );
}

fn pick(samples: &[f64], costs: &[f64]) -> Result<String, BanditMathError> {
    let names = ["a", "b", "c"];
    let candidates = costs
        .iter()
        .enumerate()
        .map(|(index, cost)| Candidate {
            model: names[index],
            cell: cell(5.0, 5.0),
            economy_score: *cost,
        })
        .collect::<Vec<_>>();
    pick_best(
        &candidates,
        weights(0.7, 0.3),
        &mut QualitySampler::Scripted(Box::new(ScriptedSampler {
            samples: samples.to_vec(),
            position: 0,
        })),
    )
}

#[rstest]
#[case::highest_sample_wins(&[0.2_f64, 0.9_f64, 0.5_f64], &[0.0_f64, 0.0_f64, 0.0_f64], "b")]
#[case::first_of_a_tie_wins(&[0.5_f64, 0.5_f64], &[0.0_f64, 0.0_f64], "a")]
#[case::cost_breaks_a_tie(&[0.5_f64, 0.5_f64], &[0.0_f64, 1.0_f64], "b")]
#[case::cost_and_quality_mix(
    &[0.2_f64, 0.95_f64, 0.4_f64],
    &[1.0_f64, 0.5_f64, 0.0_f64],
    "b"
)]
fn the_best_candidate_wins_each_draw(
    #[case] samples: &[f64],
    #[case] costs: &[f64],
    #[case] expected: &str,
) {
    assert_eq!(pick(samples, costs).unwrap(), expected);
}

#[test]
fn no_candidate_has_a_best_model() {
    let error = pick(&[], &[]).unwrap_err();
    assert_eq!(error, BanditMathError::NoModels);
    assert_eq!(error.to_string(), "pick_best called with no models");
}

#[test]
fn an_exhausted_scripted_sampler_says_so() {
    let mut sampler = QualitySampler::Scripted(Box::new(ScriptedSampler {
        samples: vec![0.25_f64, 0.75_f64],
        position: 0,
    }));
    let fifth = cell(5.0, 5.0);
    assert_eq!(format!("{:.6}", sampler.sample(fifth).unwrap()), "0.250000");
    assert_eq!(format!("{:.6}", sampler.sample(fifth).unwrap()), "0.750000");
    assert_eq!(
        sampler.sample(fifth),
        Err(BanditMathError::SamplesExhausted)
    );
}

#[test]
fn a_non_finite_scripted_sample_is_refused() {
    let mut sampler = QualitySampler::Scripted(Box::new(ScriptedSampler {
        samples: vec![f64::NAN],
        position: 0,
    }));
    assert_eq!(
        sampler.sample(cell(5.0, 5.0)),
        Err(BanditMathError::NonFinite {
            field: "scripted sample"
        })
    );
}

#[test]
fn a_seeded_beta_sampler_repeats_inside_the_unit_interval() {
    let fifth = cell(5.0, 5.0);
    let mut first = QualitySampler::Beta(Box::new(StdRng::seed_from_u64(7)));
    let mut second = QualitySampler::Beta(Box::new(StdRng::seed_from_u64(7)));
    let sample = first.sample(fifth).unwrap();
    assert_eq!(sample.to_bits(), second.sample(fifth).unwrap().to_bits());
    assert!((0.0_f64..=1.0_f64).contains(&sample));
    let next = first.sample(fifth).unwrap();
    assert!((0.0_f64..=1.0_f64).contains(&next));
    assert_ne!(sample.to_bits(), next.to_bits());
}

#[test]
fn a_seeded_beta_draw_follows_the_shape_orientation() {
    let mut low = QualitySampler::Beta(Box::new(StdRng::seed_from_u64(11)));
    let mut high = QualitySampler::Beta(Box::new(StdRng::seed_from_u64(11)));
    let low_sample = low.sample(cell(1.0, 99.0)).unwrap();
    let high_sample = high.sample(cell(99.0, 1.0)).unwrap();
    assert!(low_sample < 0.5_f64, "Beta(1, 99) drew {low_sample}");
    assert!(high_sample > 0.5_f64, "Beta(99, 1) drew {high_sample}");
}

#[test]
fn a_non_positive_shape_is_refused_by_the_beta_draw() {
    let mut sampler = QualitySampler::Beta(Box::new(StdRng::seed_from_u64(1)));
    let error = sampler.sample(cell(0.0, 5.0)).unwrap_err();
    match error {
        BanditMathError::Distribution { alpha, beta, .. } => {
            assert_eq!(alpha.to_bits(), 0.0_f64.to_bits());
            assert_eq!(beta.to_bits(), 5.0_f64.to_bits());
        }
        other @ (BanditMathError::NonFinite { .. }
        | BanditMathError::UnsupportedTier { .. }
        | BanditMathError::NoModels
        | BanditMathError::SamplesExhausted) => {
            panic!("expected a refused Beta draw, got {other:?}");
        }
    }
}

#[test]
fn the_statistical_constants_keep_the_pinned_values() {
    assert_eq!(PRIOR_MEAN_CEILING.to_bits(), 0.95_f64.to_bits());
    assert_eq!(EMPTY_CELL_MEAN.to_bits(), 0.5_f64.to_bits());
}
