use std::collections::BTreeMap;
use std::str::FromStr as _;

use crate::adaptive_classification::request_type::RequestType;
use crate::posterior_math;
use crate::tier_artifact_predictor::artifact::{TierNumber, ValidatedArtifact};
use crate::tier_artifact_predictor::error::PredictorError;

pub(crate) fn finite_posteriors(artifact: &ValidatedArtifact) -> Result<(), PredictorError> {
    for tier in TierNumber::ALL {
        let global = artifact.global.for_tier(tier);
        let global_mean = posterior_math::smoothed_mean(global.successes, global.observations);
        require_finite(
            global_mean,
            format!("the global statistic for tier {}", tier.built_in().name()),
        )?;
        let domains = domain_means(artifact, tier, global_mean)?;
        for (cohort, tiers) in &artifact.cohort {
            let Some(statistic) = tiers.get(&tier) else {
                continue;
            };
            let prior = cohort
                .split_once('|')
                .and_then(|(prefix, _rest)| RequestType::from_str(prefix).ok())
                .and_then(|request_type| domains.get(&request_type))
                .copied()
                .unwrap_or(global_mean);
            let mean = posterior_math::posterior_mean(
                statistic.successes,
                statistic.observations,
                artifact.cohort_prior_mass,
                prior,
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

// A tier's domain posterior is shared by every cohort with that request type.
fn domain_means(
    artifact: &ValidatedArtifact,
    tier: TierNumber,
    global_mean: f64,
) -> Result<BTreeMap<RequestType, f64>, PredictorError> {
    artifact
        .domain
        .iter()
        .filter_map(|(request_type, tiers)| {
            tiers.get(&tier).map(|statistic| (request_type, statistic))
        })
        .map(|(request_type, statistic)| {
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
            Ok((*request_type, mean))
        })
        .collect()
}

fn require_finite(mean: f64, statistic: String) -> Result<(), PredictorError> {
    if mean.is_finite() {
        return Ok(());
    }
    Err(PredictorError::NonFinitePosterior { statistic })
}
