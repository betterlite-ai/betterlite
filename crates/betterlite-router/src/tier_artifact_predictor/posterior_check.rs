use std::str::FromStr as _;

use crate::adaptive_classification::request_type::RequestType;
use crate::posterior_math;
use crate::tier_artifact_predictor::artifact::{TierNumber, ValidatedArtifact};
use crate::tier_artifact_predictor::error::PredictorError;

// every posterior mean the artifact can produce, so a loaded artifact never yields a NaN forecast
pub(crate) fn finite_posteriors(artifact: &ValidatedArtifact) -> Result<(), PredictorError> {
    for tier in TierNumber::ALL {
        let global = artifact.global.for_tier(tier);
        let mean = posterior_math::smoothed_mean(global.successes, global.observations);
        require_finite(
            mean,
            format!("the global statistic for tier {}", tier.built_in().name()),
        )?;
    }
    for (request_type, tiers) in &artifact.domain {
        for (tier, statistic) in tiers {
            let global = artifact.global.for_tier(*tier);
            let global_mean = posterior_math::smoothed_mean(global.successes, global.observations);
            let mean = posterior_math::posterior_mean(
                statistic.successes,
                statistic.observations,
                artifact.domain_prior_mass,
                global_mean,
            );
            require_finite(
                mean,
                format!(
                    "the domain statistic for {request_type} at tier {}",
                    tier.built_in().name()
                ),
            )?;
        }
    }
    finite_cohorts(artifact)
}

fn finite_cohorts(artifact: &ValidatedArtifact) -> Result<(), PredictorError> {
    for (cohort, tiers) in &artifact.cohort {
        let request_type = cohort
            .split_once('|')
            .and_then(|(prefix, _rest)| RequestType::from_str(prefix).ok());
        for (tier, statistic) in tiers {
            let global = artifact.global.for_tier(*tier);
            let global_mean = posterior_math::smoothed_mean(global.successes, global.observations);
            let domain_mean = request_type
                .and_then(|domain_type| artifact.domain.get(&domain_type))
                .and_then(|domain_tiers| domain_tiers.get(tier))
                .map_or(global_mean, |domain| {
                    posterior_math::posterior_mean(
                        domain.successes,
                        domain.observations,
                        artifact.domain_prior_mass,
                        global_mean,
                    )
                });
            let mean = posterior_math::posterior_mean(
                statistic.successes,
                statistic.observations,
                artifact.cohort_prior_mass,
                domain_mean,
            );
            require_finite(
                mean,
                format!(
                    "the cohort statistic {cohort} at tier {}",
                    tier.built_in().name()
                ),
            )?;
        }
    }
    Ok(())
}

fn require_finite(mean: f64, statistic: String) -> Result<(), PredictorError> {
    if mean.is_finite() {
        return Ok(());
    }
    Err(PredictorError::NonFinitePosterior { statistic })
}
