use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_config::sections::tier_artifact_predictor::artifact::{
    Artifact, CohortStatistic, Dataset, DomainStatistic, GlobalStatistic,
};

use crate::adaptive_classification::request_type::RequestType;
use crate::tier_artifact_predictor::artifact::{
    GlobalStatistics, TierNumber, TierStatistic, TierStatistics, ValidatedArtifact,
};
use crate::tier_artifact_predictor::error::PredictorError;

// TrainedTierArtifact's validators, in the reference's field order
pub(crate) fn validate(artifact: &Artifact) -> Result<ValidatedArtifact, PredictorError> {
    if artifact.schema_version != 1 {
        return Err(PredictorError::SchemaVersion {
            version: artifact.schema_version,
        });
    }
    let global = global_statistics(&artifact.global_statistics)?;
    let domain = domain_statistics(&artifact.domain_statistics)?;
    let cohort = cohort_statistics(&artifact.cohort_statistics)?;
    let domain_prior_mass = prior_mass(artifact.domain_prior_mass, "domain_prior_mass")?;
    let cohort_prior_mass = prior_mass(artifact.cohort_prior_mass, "cohort_prior_mass")?;
    let routing_threshold = threshold(artifact.routing_threshold)?;
    let datasets = datasets(&artifact.datasets)?;
    require_text(&artifact.success_definition, "success_definition")?;
    require_text(&artifact.split_method, "split_method")?;
    Ok(ValidatedArtifact {
        global,
        domain,
        cohort,
        domain_prior_mass,
        cohort_prior_mass,
        routing_threshold,
        datasets,
        success_definition: artifact.success_definition.clone(),
        split_method: artifact.split_method.clone(),
    })
}

// _statistics_are_unique: each tier appears exactly once, and a repeated tier is refused rather than overwritten
fn global_statistics(statistics: &[GlobalStatistic]) -> Result<GlobalStatistics, PredictorError> {
    let mut by_tier = BTreeMap::new();
    for (index, statistic) in statistics.iter().enumerate() {
        let tier = TierNumber::declared(statistic.tier)?;
        let field = format!("global_statistics[{index}]");
        let value = statistic_of(statistic.successes, statistic.observations, &field)?;
        if by_tier.insert(tier, value).is_some() {
            return Err(PredictorError::DuplicateGlobalTier { tier });
        }
    }
    match (
        by_tier.remove(&TierNumber::One),
        by_tier.remove(&TierNumber::Two),
        by_tier.remove(&TierNumber::Three),
        by_tier.remove(&TierNumber::Four),
    ) {
        (Some(one), Some(two), Some(three), Some(four)) => Ok(GlobalStatistics {
            one,
            two,
            three,
            four,
        }),
        _ => Err(PredictorError::MissingGlobalStatistics),
    }
}

fn domain_statistics(
    statistics: &[DomainStatistic],
) -> Result<BTreeMap<RequestType, TierStatistics>, PredictorError> {
    let mut by_type: BTreeMap<RequestType, TierStatistics> = BTreeMap::new();
    for (index, statistic) in statistics.iter().enumerate() {
        let request_type = RequestType::from_str(&statistic.request_type).map_err(|_error| {
            PredictorError::UnknownRequestType {
                request_type: statistic.request_type.clone(),
            }
        })?;
        let tier = TierNumber::declared(statistic.tier)?;
        let field = format!("domain_statistics[{index}]");
        let value = statistic_of(statistic.successes, statistic.observations, &field)?;
        let tiers = by_type.entry(request_type).or_default();
        if tiers.insert(tier, value).is_some() {
            return Err(PredictorError::DuplicateDomainStatistic {
                request_type: statistic.request_type.clone(),
                tier,
            });
        }
    }
    Ok(by_type)
}

fn cohort_statistics(
    statistics: &[CohortStatistic],
) -> Result<BTreeMap<String, TierStatistics>, PredictorError> {
    let mut by_cohort: BTreeMap<String, TierStatistics> = BTreeMap::new();
    for (index, statistic) in statistics.iter().enumerate() {
        require_text(
            &statistic.cohort,
            &format!("cohort_statistics[{index}].cohort"),
        )?;
        let tier = TierNumber::declared(statistic.tier)?;
        let field = format!("cohort_statistics[{index}]");
        let value = statistic_of(statistic.successes, statistic.observations, &field)?;
        let tiers = by_cohort.entry(statistic.cohort.clone()).or_default();
        if tiers.insert(tier, value).is_some() {
            return Err(PredictorError::DuplicateCohortStatistic {
                cohort: statistic.cohort.clone(),
                tier,
            });
        }
    }
    Ok(by_cohort)
}

// the source-data attribution survives validation, and a dataset names where the statistics came from
fn datasets(items: &[Dataset]) -> Result<Vec<Dataset>, PredictorError> {
    for (index, dataset) in items.iter().enumerate() {
        require_text(&dataset.name, &format!("datasets[{index}].name"))?;
        require_text(&dataset.url, &format!("datasets[{index}].url"))?;
        require_text(&dataset.license, &format!("datasets[{index}].license"))?;
        require_text(
            &dataset.success_definition,
            &format!("datasets[{index}].success_definition"),
        )?;
        if dataset.rows == 0 {
            return Err(PredictorError::EmptyDatasetRows { index });
        }
    }
    Ok(items.to_vec())
}

fn statistic_of(
    successes: f64,
    observations: f64,
    field: &str,
) -> Result<TierStatistic, PredictorError> {
    if !successes.is_finite() {
        return Err(PredictorError::NonFinite {
            field: format!("{field}.successes"),
        });
    }
    if !observations.is_finite() {
        return Err(PredictorError::NonFinite {
            field: format!("{field}.observations"),
        });
    }
    if successes < 0.0_f64 {
        return Err(PredictorError::NegativeSuccesses {
            field: format!("{field}.successes"),
            value: successes,
        });
    }
    if observations <= 0.0_f64 {
        return Err(PredictorError::NonPositiveObservations {
            field: format!("{field}.observations"),
            value: observations,
        });
    }
    if successes > observations {
        return Err(PredictorError::SuccessesExceedObservations {
            field: field.to_owned(),
            successes,
            observations,
        });
    }
    Ok(TierStatistic {
        successes,
        observations,
    })
}

fn prior_mass(value: f64, field: &str) -> Result<f64, PredictorError> {
    if !value.is_finite() {
        return Err(PredictorError::NonFinite {
            field: field.to_owned(),
        });
    }
    if value <= 0.0_f64 {
        return Err(PredictorError::PriorMass {
            field: field.to_owned(),
            mass: value,
        });
    }
    Ok(value)
}

fn threshold(value: f64) -> Result<f64, PredictorError> {
    if !(0.0_f64..=1.0_f64).contains(&value) {
        return Err(PredictorError::ThresholdRange { threshold: value });
    }
    Ok(value)
}

fn require_text(value: &str, field: &str) -> Result<(), PredictorError> {
    if value.is_empty() {
        return Err(PredictorError::EmptyField {
            field: field.to_owned(),
        });
    }
    Ok(())
}
