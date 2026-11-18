#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::tier_artifact_predictor as section;
use rstest::rstest;

use crate::tier_artifact_predictor::artifact::{
    GlobalStatistics, TierStatistic, ValidatedArtifact, parse_artifact,
};
use crate::tier_artifact_predictor::predictor::TierSuccessPredictor;

mod tier_artifact_predictor_behaviors;
mod tier_artifact_predictor_boundaries;

pub(crate) fn statistics(
    one: (f64, f64),
    two: (f64, f64),
    three: (f64, f64),
    four: (f64, f64),
) -> GlobalStatistics {
    let tier = |(successes, observations)| TierStatistic {
        successes,
        observations,
    };
    GlobalStatistics {
        one: tier(one),
        two: tier(two),
        three: tier(three),
        four: tier(four),
    }
}

pub(crate) fn artifact_with(global: GlobalStatistics) -> ValidatedArtifact {
    ValidatedArtifact {
        global,
        domain: BTreeMap::new(),
        cohort: BTreeMap::new(),
        domain_prior_mass: 200.0,
        cohort_prior_mass: 20.0,
        routing_threshold: 0.75,
        datasets: Vec::new(),
        success_definition: "quality score meets the dataset success threshold".to_owned(),
        split_method: "sha256(prompt): 70% train, 15% validation, 15% test".to_owned(),
    }
}

pub(crate) fn predictor(global: GlobalStatistics) -> TierSuccessPredictor {
    TierSuccessPredictor::new(artifact_with(global), None).expect("the pinned patterns compile")
}

// the four statistics as an artifact object spells them
pub(crate) fn global_json(statistics: [(f64, f64); 4]) -> String {
    statistics
        .into_iter()
        .enumerate()
        .map(|(index, (successes, observations))| {
            let tier = index.saturating_add(1);
            format!(
                "{{\"tier\": {tier}, \"successes\": {successes}, \"observations\": {observations}}}"
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn artifact_json(global: &str, fields: &str) -> String {
    format!("{{\"global_statistics\": [{global}]{fields}}}")
}

pub(crate) fn parsed(global: &str, fields: &str) -> ValidatedArtifact {
    parse_artifact(artifact_json(global, fields).as_bytes())
        .expect("the crafted artifact validates")
}

pub(crate) fn texts(probabilities: &[f64; 4]) -> Vec<String> {
    probabilities
        .iter()
        .map(|probability| format!("{probability:.6}"))
        .collect()
}

// the config seed this crate's predictor is built from, resolved as the server resolves it
pub(crate) const CONFIG_SEED: &str = include_str!(
    "../../../betterlite-config/fuzz/corpus/config_yaml/tier-artifact-predictor-valid.yaml"
);

pub(crate) fn corpus_config(source: &str) -> Result<section::Config, String> {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map_err(|error| error.to_string())?;
    config
        .sections
        .tier_artifact_predictor
        .deployments
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "the seed configures no tier artifact predictor".to_owned())
}

#[rstest]
fn replays_the_tier_data_corpus(#[files("fuzz/corpus/tier_data/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = parse_artifact(&bytes);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} must not decode");
    } else {
        assert!(outcome.is_ok(), "{name} must decode");
    }
}
