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
use crate::tier_artifact_predictor::posterior_check;

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
    let validated = ValidatedArtifact {
        global,
        domain,
        cohort,
        domain_prior_mass,
        cohort_prior_mass,
        routing_threshold,
        datasets,
        success_definition: artifact.success_definition.clone(),
        split_method: artifact.split_method.clone(),
    };
    posterior_check::finite_posteriors(&validated)?;
    Ok(validated)
}

trait DeclaredStatistic {
    fn declared(&self) -> (u32, f64, f64);
}

macro_rules! declared_statistic {
    ($statistic:ty) => {
        impl DeclaredStatistic for $statistic {
            fn declared(&self) -> (u32, f64, f64) {
                (self.tier, self.successes, self.observations)
            }
        }
    };
}

declared_statistic!(GlobalStatistic);
declared_statistic!(DomainStatistic);
declared_statistic!(CohortStatistic);

fn parse_statistic<S>(
    statistic: &S,
    field: &str,
) -> Result<(TierNumber, TierStatistic), PredictorError>
where
    S: DeclaredStatistic,
{
    let (tier, successes, observations) = statistic.declared();
    Ok((
        TierNumber::declared(tier)?,
        statistic_of(successes, observations, field)?,
    ))
}

fn statistics_by_key<S, K, F>(
    statistics: &[S],
    list: &str,
    mut statistic_key: F,
) -> Result<BTreeMap<K, TierStatistics>, PredictorError>
where
    S: DeclaredStatistic,
    K: Ord,
    F: FnMut(&S, &str) -> Result<K, PredictorError>,
{
    let mut by_key: BTreeMap<K, TierStatistics> = BTreeMap::new();
    for (index, statistic) in statistics.iter().enumerate() {
        let field = format!("{list}[{index}]");
        let key = statistic_key(statistic, &field)?;
        let (tier, value) = parse_statistic(statistic, &field)?;
        if by_key.entry(key).or_default().insert(tier, value).is_some() {
            return Err(PredictorError::DuplicateStatistic {
                field,
                tier: tier.built_in().name(),
            });
        }
    }
    Ok(by_key)
}

fn global_statistics(statistics: &[GlobalStatistic]) -> Result<GlobalStatistics, PredictorError> {
    let mut by_tier: BTreeMap<TierNumber, TierStatistic> = BTreeMap::new();
    for (index, statistic) in statistics.iter().enumerate() {
        let field = format!("global_statistics[{index}]");
        let (tier, value) = parse_statistic(statistic, &field)?;
        if by_tier.insert(tier, value).is_some() {
            return Err(PredictorError::DuplicateStatistic {
                field,
                tier: tier.built_in().name(),
            });
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
    statistics_by_key(statistics, "domain_statistics", |statistic, _field| {
        RequestType::from_str(&statistic.request_type).map_err(|_error| {
            PredictorError::UnknownRequestType {
                request_type: statistic.request_type.clone(),
            }
        })
    })
}

fn cohort_statistics(
    statistics: &[CohortStatistic],
) -> Result<BTreeMap<String, TierStatistics>, PredictorError> {
    statistics_by_key(statistics, "cohort_statistics", |statistic, field| {
        Ok(require_text(&statistic.cohort, &format!("{field}.cohort"))?.to_owned())
    })
}

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

fn require_text<'text>(value: &'text str, field: &str) -> Result<&'text str, PredictorError> {
    if value.is_empty() {
        return Err(PredictorError::EmptyField {
            field: field.to_owned(),
        });
    }
    Ok(value)
}
