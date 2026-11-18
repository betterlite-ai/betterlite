use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::custom_dimensions as section;

use self::ClassifierType::{Heuristic, HeuristicFirst, Hybrid};
use crate::complexity_scoring::dimensions::{CustomDimension, ScoringMode};

mod tests;

pub const MAX_CUSTOM_DIMENSIONS: usize = 16;
pub const MAX_CUSTOM_DIMENSIONS_WORK: u64 = 8192;
const BUILT_IN_DIMENSIONS: [&str; 7] = [
    "tokenCount",
    "codePresence",
    "reasoningMarkers",
    "technicalTerms",
    "simpleIndicators",
    "multiStepPatterns",
    "questionComplexity",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum ClassifierType {
    Heuristic,
    HeuristicV2,
    Llm,
    Capability,
    LlmV2,
    Custom,
    HeuristicFirst,
    Hybrid,
    Jev,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DimensionsError {
    #[error("classifier_type {value:?} is not a known classifier type")]
    ClassifierType { value: String },
    #[error(transparent)]
    Dimension(#[from] section::DimensionProblem),
    #[error("custom_dimensions requires classifier_type heuristic, heuristic_first or hybrid")]
    Classifier,
    #[error("custom dimension names must be unique and must not shadow built-in dimensions")]
    Names,
    #[error("custom dimension weights must be inline, not in dimension_weights")]
    Weights,
    #[error(
        "custom_dimensions regex work estimate is {work}; the limit across the router is {limit}"
    )]
    Work { work: u64, limit: u64 },
    #[error("custom_dimensions allows at most {limit} dimensions, not {count}")]
    Count { count: usize, limit: usize },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Validated {
    pub dimensions: Vec<CustomDimension>,
    pub work: u64,
}

pub fn validate(
    declared: &[section::Config],
    classifier: &str,
    dimension_weights: &BTreeMap<String, f64>,
) -> Result<Validated, DimensionsError> {
    let mut checked = Vec::with_capacity(declared.len());
    for config in declared {
        checked.push(section::validate_dimension(config)?);
    }
    if checked.len() > MAX_CUSTOM_DIMENSIONS {
        return Err(DimensionsError::Count {
            count: checked.len(),
            limit: MAX_CUSTOM_DIMENSIONS,
        });
    }
    let classifier_type = classifier.parse::<ClassifierType>().map_err(|_source| {
        DimensionsError::ClassifierType {
            value: classifier.to_owned(),
        }
    })?;
    if checked.is_empty() {
        return Ok(Validated::default());
    }
    if !matches!(classifier_type, Heuristic | HeuristicFirst | Hybrid) {
        return Err(DimensionsError::Classifier);
    }
    validate_names(&checked, dimension_weights)?;
    let work = checked
        .iter()
        .fold(0_u64, |work, dimension| work.saturating_add(dimension.work));
    if work > MAX_CUSTOM_DIMENSIONS_WORK {
        return Err(DimensionsError::Work {
            work,
            limit: MAX_CUSTOM_DIMENSIONS_WORK,
        });
    }
    let dimensions = checked.into_iter().map(convert).collect();
    Ok(Validated { dimensions, work })
}

fn validate_names(
    checked: &[section::Checked],
    dimension_weights: &BTreeMap<String, f64>,
) -> Result<(), DimensionsError> {
    let mut names = BTreeSet::new();
    for dimension in checked {
        let folded = caseless::default_case_fold_str(&dimension.name);
        if BUILT_IN_DIMENSIONS
            .iter()
            .any(|name| caseless::default_case_fold_str(name) == folded)
            || !names.insert(folded)
        {
            return Err(DimensionsError::Names);
        }
    }
    if dimension_weights
        .keys()
        .any(|name| names.contains(&caseless::default_case_fold_str(name)))
    {
        return Err(DimensionsError::Weights);
    }
    Ok(())
}

fn convert(checked: section::Checked) -> CustomDimension {
    CustomDimension {
        name: checked.name,
        weight: checked.weight,
        keywords: checked.keywords,
        patterns: checked.matcher_patterns,
        scoring_mode: match checked.scoring_mode {
            section::ScoringMode::Binary => ScoringMode::Binary,
            section::ScoringMode::MatchCount => ScoringMode::MatchCount,
        },
    }
}
