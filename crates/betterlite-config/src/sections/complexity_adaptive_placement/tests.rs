#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::adaptive_classification::Weights;
use crate::sections::complexity_adaptive_placement::{AdaptiveEligible, Config, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.complexity_adaptive_placement)
        .map_err(|error| error.to_string())
}

fn router(fields: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n{fields}"
    )
}

fn deployment(resolved: &Settings) -> Config {
    resolved.deployments[0]
        .clone()
        .expect("the deployment configures complexity_router_config")
}

#[test]
fn a_deployment_without_the_key_keeps_its_index() {
    let source =
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![None],
        }
    );
}

#[test]
fn an_empty_router_config_resolves_to_the_pinned_defaults() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config: {}\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Some(Config {
            adaptive: false,
            weights: None,
            tier_distance_penalty: 0.5,
            eligible: AdaptiveEligible::All,
        })]
    );
}

#[test]
fn declared_values_replace_the_defaults() {
    let source = router(
        "        adaptive: true\n        adaptive_weights:\n          quality: 0.4\n          cost: 0.6\n        tier_distance_penalty: 0.25\n        adaptive_eligible: classified_tier\n",
    );
    let resolved = deployment(&settings(&source, &[]).unwrap());
    assert!(resolved.adaptive);
    assert_eq!(
        resolved.weights,
        Some(Weights {
            quality: Some(0.4_f64),
            economy: Some(0.6_f64),
        })
    );
    assert_eq!(resolved.tier_distance_penalty.to_bits(), 0.25_f64.to_bits());
    assert_eq!(resolved.eligible, AdaptiveEligible::ClassifiedTier);
}

// {} keeps the AdaptiveRouterWeights model defaults 0.7/0.3, unlike the absent key's 0.3/0.7
#[test]
fn an_empty_weights_object_keeps_the_weight_model_defaults() {
    let source = router("        adaptive_weights: {}\n");
    let resolved = deployment(&settings(&source, &[]).unwrap());
    assert_eq!(
        resolved.weights,
        Some(Weights {
            quality: None,
            economy: None,
        })
    );
}

#[test]
fn an_environment_reference_resolves_into_the_declared_type() {
    let source = router(
        "        adaptive: os.environ/ADAPTIVE\n        tier_distance_penalty: os.environ/PENALTY\n        adaptive_eligible: os.environ/ELIGIBLE\n",
    );
    let resolved = deployment(
        &settings(
            &source,
            &[
                ("ADAPTIVE", "yes"),
                ("PENALTY", "0.75"),
                ("ELIGIBLE", "classified_tier"),
            ],
        )
        .unwrap(),
    );
    assert!(resolved.adaptive);
    assert_eq!(resolved.tier_distance_penalty.to_bits(), 0.75_f64.to_bits());
    assert_eq!(resolved.eligible, AdaptiveEligible::ClassifiedTier);
}

#[test]
fn a_declared_or_environment_false_stays_false() {
    let literal = router("        adaptive: false\n");
    assert!(!deployment(&settings(&literal, &[]).unwrap()).adaptive);
    let referenced = router("        adaptive: os.environ/ADAPTIVE\n");
    let resolved = settings(&referenced, &[("ADAPTIVE", "false")]).unwrap();
    assert!(!deployment(&resolved).adaptive);
}

#[test]
fn an_empty_environment_value_takes_the_default() {
    let source = router("        tier_distance_penalty: os.environ/PENALTY\n");
    let resolved = deployment(&settings(&source, &[("PENALTY", "")]).unwrap());
    assert_eq!(resolved.tier_distance_penalty.to_bits(), 0.5_f64.to_bits());
}

#[test]
fn a_missing_environment_value_is_an_error_that_names_it() {
    let source = router("        tier_distance_penalty: os.environ/PENALTY\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("PENALTY"), "{error}");
}

#[test]
fn a_negative_or_non_finite_penalty_is_rejected() {
    let negative = router("        tier_distance_penalty: -0.1\n");
    let error = settings(&negative, &[]).unwrap_err();
    assert!(error.contains("tier_distance_penalty"), "{error}");
    let infinite = router("        tier_distance_penalty: .inf\n");
    let infinite_error = settings(&infinite, &[]).unwrap_err();
    assert!(
        infinite_error.contains("tier_distance_penalty"),
        "{infinite_error}"
    );
}

#[test]
fn an_unknown_eligible_mode_is_rejected() {
    let source = router("        adaptive_eligible: every_tier\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("adaptive_eligible"), "{error}");
}

#[test]
fn the_router_sibling_keys_stay_supported() {
    let source = router("        adaptive: true\n        tiers:\n          simple: mini\n");
    let resolved = deployment(&settings(&source, &[]).unwrap());
    assert!(resolved.adaptive);
}
