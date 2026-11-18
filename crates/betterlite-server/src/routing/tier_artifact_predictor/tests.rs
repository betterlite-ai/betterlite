#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::tier_artifact_predictor as section;
use betterlite_config::sections::tier_artifact_predictor::artifact::{Artifact, GlobalStatistic};
use betterlite_router::complexity_scoring::tier::{ComplexityTier, Tier};
use betterlite_router::tier_artifact_predictor::error::PredictorError;

use super::{deployment_predictors, predictor};

const CONFIG_SEED: &str = include_str!(
    "../../../../betterlite-config/fuzz/corpus/config_yaml/tier-artifact-predictor-valid.yaml"
);

fn inline_artifact() -> Artifact {
    let global_statistics = [1_u32, 2, 3, 4]
        .into_iter()
        .zip([0.0_f64, 1.0_f64, 2.0_f64, 3.0_f64])
        .map(|(tier, successes)| GlobalStatistic {
            tier,
            successes,
            observations: 8.0,
        })
        .collect();
    Artifact {
        schema_version: 1,
        global_statistics,
        domain_statistics: Vec::new(),
        cohort_statistics: Vec::new(),
        domain_prior_mass: 200.0,
        cohort_prior_mass: 20.0,
        routing_threshold: 0.75,
        datasets: Vec::new(),
        success_definition: "s".to_owned(),
        split_method: "m".to_owned(),
    }
}

fn seed_config() -> section::Config {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(CONFIG_SEED)
    .expect("the shipped seed loads");
    config
        .sections
        .tier_artifact_predictor
        .deployments
        .into_iter()
        .flatten()
        .next()
        .expect("the shipped seed configures the predictor")
}

#[test]
fn the_shipped_seed_builds_a_predictor_once() {
    let configured = seed_config();
    let predictor = predictor(&configured).expect("the packaged artifact validates");
    let outcome = predictor.classify("Hello.");
    assert_eq!(outcome.tier, Tier::BuiltIn(ComplexityTier::Simple));
    assert_eq!(outcome.forecast.threshold.to_bits(), 0.875_f64.to_bits());
}

#[test]
fn an_inline_artifact_builds_a_predictor() {
    let configured = section::Config {
        artifact: section::ArtifactSource::Inline(Box::new(inline_artifact())),
        success_threshold: Some(0.5_f64),
    };
    let predictor = predictor(&configured).expect("the inline artifact validates");
    assert_eq!(
        predictor.classify("Hello.").forecast.threshold.to_bits(),
        0.5_f64.to_bits()
    );
}

#[test]
fn deployments_stay_aligned_with_the_model_list() {
    let settings = section::Settings {
        deployments: vec![None, Some(seed_config())],
    };
    let predictors = deployment_predictors(&settings).expect("the configured deployment validates");
    assert_eq!(predictors.len(), 2);
    assert!(predictors[0].is_none());
    assert!(predictors[1].is_some());
}

#[test]
fn an_unknown_artifact_is_refused_before_any_state_is_published() {
    let configured = section::Config {
        artifact: section::ArtifactSource::BuiltIn("arc_agi".to_owned()),
        success_threshold: None,
    };
    assert!(matches!(
        predictor(&configured),
        Err(PredictorError::UnknownArtifact { name }) if name == "arc_agi"
    ));
}

#[test]
fn an_invalid_inline_artifact_is_refused() {
    let mut artifact = inline_artifact();
    artifact.global_statistics.truncate(3);
    let configured = section::Config {
        artifact: section::ArtifactSource::Inline(Box::new(artifact)),
        success_threshold: None,
    };
    assert!(matches!(
        predictor(&configured),
        Err(PredictorError::MissingGlobalStatistics)
    ));
}
