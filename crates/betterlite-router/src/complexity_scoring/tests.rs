#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::complexity_scoring as section;

use crate::complexity_scoring::classify::{ClassificationOutcome, PromptInput, Scorer};
use crate::complexity_scoring::config::Config;
use crate::complexity_scoring::error::ScoringError;

mod complexity_scoring_behaviors;
mod complexity_scoring_boundaries;

const CONFIG_VALID: &str = include_str!(
    "../../../betterlite-config/fuzz/corpus/config_yaml/complexity-scoring-valid.yaml"
);
const CONFIG_INVALID: &str = include_str!(
    "../../../betterlite-config/fuzz/corpus/config_yaml/complexity-scoring-invalid.yaml"
);

// the section view of a deployment that configures nothing, so a test names only the keys it sets
pub(crate) fn blank() -> section::Config {
    section::Config {
        tier_models: None,
        complexity_thresholds: None,
        dimension_weights: None,
        tier_boundaries: None,
        reasoning_override_min_score: None,
        code_keywords: None,
        reasoning_keywords: None,
        technical_keywords: None,
        custom_technical_keywords: None,
        simple_keywords: None,
        default_model: None,
    }
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn weights(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs
        .iter()
        .map(|(name, weight)| ((*name).to_owned(), *weight))
        .collect()
}

pub(crate) fn tiers(pairs: &[(&str, &[&str])]) -> BTreeMap<String, section::TierModelValue> {
    pairs
        .iter()
        .map(|(tier, models)| {
            let value = match models {
                [model] => section::TierModelValue::Single((*model).to_owned()),
                models => section::TierModelValue::Pool(strings(models)),
            };
            ((*tier).to_owned(), value)
        })
        .collect()
}

pub(crate) fn resolve(input: &section::Config) -> Result<Config, ScoringError> {
    Config::resolve(input)
}

pub(crate) fn resolved(input: &section::Config) -> Config {
    resolve(input).expect("the test config resolves")
}

pub(crate) fn scorer(input: &section::Config) -> Scorer {
    Scorer::new(resolved(input)).expect("the test keywords and patterns compile")
}

pub(crate) fn classify(
    input: &section::Config,
    prompt: &str,
    system_prompt: Option<&str>,
) -> ClassificationOutcome {
    scorer(input)
        .score_and_classify(&PromptInput {
            prompt,
            system_prompt,
        })
        .expect("the test prompt scores")
}

// six decimals, the precision the probe wire carries
pub(crate) fn fixed(score: f64) -> String {
    format!("{score:.6}")
}

// the router config of the one deployment in a corpus seed, resolved as the server resolves it
pub(crate) fn corpus_config(source: &str) -> Result<Config, String> {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map_err(|error| error.to_string())?;
    let configured = config
        .sections
        .complexity_scoring
        .deployments
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "the seed configures no complexity router".to_owned())?;
    resolve(&configured).map_err(|error| error.to_string())
}
