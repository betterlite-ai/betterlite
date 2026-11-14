#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::adaptive_classification::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.adaptive_classification)
        .map_err(|error| error.to_string())
}

const ROUTER: &str = "\
model_list:
  - model_name: smart-cheap-router
    litellm_params:
      adaptive_router_config:
        available_models: [fast, smart]
        weights:
          quality: 0.6
          cost: 0.4
  - model_name: fast
    litellm_params:
      model: openai/gpt-4o-mini
    model_info:
      adaptive_router_preferences:
        quality_tier: 2
        strengths: [code_generation, technical_design]
  - model_name: plain
    litellm_params:
      model: openai/gpt-4o
";

#[test]
fn a_deployment_without_the_keys_keeps_its_index() {
    let resolved = settings(
        "model_list:\n  - model_name: plain\n    litellm_params:\n      model: openai/gpt-4o\n",
        &[],
    )
    .unwrap();
    assert_eq!(resolved.deployments, vec![None]);
}

#[test]
fn a_full_configuration_resolves_every_key() {
    let resolved = settings(ROUTER, &[]).unwrap();
    assert_eq!(resolved.deployments.len(), 3);
    let router = resolved.deployments[0].as_ref().unwrap();
    assert_eq!(router.model_name, "smart-cheap-router");
    let config = router.config.as_ref().unwrap();
    assert_eq!(
        config.available_models,
        vec!["fast".to_owned(), "smart".to_owned()]
    );
    let weights = config.weights.as_ref().unwrap();
    assert_eq!(weights.quality.unwrap().to_bits(), 0.6_f64.to_bits());
    assert_eq!(weights.economy.unwrap().to_bits(), 0.4_f64.to_bits());
    assert_eq!(router.preferences, None);
    let fast = resolved.deployments[1].as_ref().unwrap();
    assert_eq!(fast.model_name, "fast");
    assert_eq!(fast.config, None);
    let preferences = fast.preferences.as_ref().unwrap();
    assert_eq!(preferences.quality_tier, 2);
    assert_eq!(
        preferences.strengths,
        vec!["code_generation".to_owned(), "technical_design".to_owned()]
    );
    assert_eq!(resolved.deployments[2], None);
}

#[test]
fn the_declared_defaults_fill_the_missing_keys() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      adaptive_router_config:
        available_models: [fast]
  - model_name: fast
    model_info:
      adaptive_router_preferences:
        quality_tier: 3
";
    let resolved = settings(source, &[]).unwrap();
    let config = resolved.deployments[0]
        .as_ref()
        .unwrap()
        .config
        .as_ref()
        .unwrap();
    assert_eq!(config.weights, None);
    let preferences = resolved.deployments[1]
        .as_ref()
        .unwrap()
        .preferences
        .as_ref()
        .unwrap();
    assert_eq!(preferences.quality_tier, 3);
    assert_eq!(preferences.strengths, Vec::<String>::new());
}

#[test]
fn environment_references_resolve_once_at_load() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      adaptive_router_config:
        available_models: [os.environ/FAST, os.environ/SMART]
        weights:
          quality: os.environ/QUALITY
          cost: os.environ/COST
  - model_name: fast
    model_info:
      adaptive_router_preferences:
        quality_tier: os.environ/TIER
        strengths: [os.environ/STRENGTH]
";
    let resolved = settings(
        source,
        &[
            ("FAST", "fast"),
            ("SMART", "smart"),
            ("QUALITY", "0.6"),
            ("COST", "0.4"),
            ("TIER", "3"),
            ("STRENGTH", "writing"),
        ],
    )
    .unwrap();
    let config = resolved.deployments[0]
        .as_ref()
        .unwrap()
        .config
        .as_ref()
        .unwrap();
    assert_eq!(
        config.available_models,
        vec!["fast".to_owned(), "smart".to_owned()]
    );
    let weights = config.weights.as_ref().unwrap();
    assert_eq!(weights.quality.unwrap().to_bits(), 0.6_f64.to_bits());
    assert_eq!(weights.economy.unwrap().to_bits(), 0.4_f64.to_bits());
    let preferences = resolved.deployments[1]
        .as_ref()
        .unwrap()
        .preferences
        .as_ref()
        .unwrap();
    assert_eq!(preferences.quality_tier, 3);
    assert_eq!(preferences.strengths, vec!["writing".to_owned()]);
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      adaptive_router_config:
        available_models: [fast]
        weights:
          quality: os.environ/QUALITY
";
    let resolved = settings(source, &[("QUALITY", " ")]).unwrap();
    let config = resolved.deployments[0]
        .as_ref()
        .unwrap()
        .config
        .as_ref()
        .unwrap();
    let weights = config.weights.as_ref().unwrap();
    assert_eq!(weights.quality, None);
}

#[test]
fn a_missing_environment_reference_names_the_key() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      adaptive_router_config:
        available_models: [fast]
        weights:
          quality: os.environ/ABSENT
";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("ABSENT"), "{error}");
    assert!(
        error.contains("litellm_params.adaptive_router_config.weights.quality"),
        "{error}"
    );
}

#[rstest]
#[case::quoted("quality: \"0.6\"", "weights.quality")]
#[case::text("quality: heavy", "weights.quality")]
#[case::boolean("cost: true", "weights.cost")]
fn a_wrongly_typed_weight_is_refused(#[case] weight: &str, #[case] fragment: &str) {
    let source = format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      adaptive_router_config:\n        available_models: [fast]\n        weights:\n          {weight}\n"
    );
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains(fragment), "{error}");
}

#[rstest]
#[case::quoted("quality_tier: \"2\"")]
#[case::boolean("quality_tier: true")]
#[case::fractional("quality_tier: 2.5")]
fn a_wrongly_typed_quality_tier_is_refused(#[case] tier: &str) {
    let source = format!(
        "model_list:\n  - model_name: fast\n    model_info:\n      adaptive_router_preferences:\n        {tier}\n"
    );
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("quality_tier"), "{error}");
}

#[test]
fn a_preferences_block_without_a_quality_tier_is_refused() {
    let source = "model_list:\n  - model_name: fast\n    model_info:\n      adaptive_router_preferences:\n        strengths: []\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("quality_tier"), "{error}");
}

#[test]
fn a_router_config_without_available_models_is_refused() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      adaptive_router_config:\n        weights:\n          quality: 0.7\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("available_models"), "{error}");
}

#[test]
fn an_unknown_config_key_is_unsupported() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      adaptive_router_config:\n        available_models: [fast]\n        fallback_model: slow\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(
        error.contains("adaptive_router_config.fallback_model"),
        "{error}"
    );
}

#[test]
fn an_unknown_preference_key_is_unsupported() {
    let source = "model_list:\n  - model_name: fast\n    model_info:\n      adaptive_router_preferences:\n        quality_tier: 2\n        confidence: 0.9\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(
        error.contains("adaptive_router_preferences.confidence"),
        "{error}"
    );
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/adaptive-classification-valid.yaml");
    let resolved = settings(source, &[]).unwrap();
    let config = resolved.deployments[0]
        .as_ref()
        .unwrap()
        .config
        .as_ref()
        .unwrap();
    assert_eq!(config.available_models, vec!["fast".to_owned()]);
    let weights = config.weights.as_ref().unwrap();
    assert_eq!(weights.quality.unwrap().to_bits(), 0.6_f64.to_bits());
    assert_eq!(weights.economy.unwrap().to_bits(), 0.4_f64.to_bits());
    let preferences = resolved.deployments[1]
        .as_ref()
        .unwrap()
        .preferences
        .as_ref()
        .unwrap();
    assert_eq!(preferences.quality_tier, 3);
    assert_eq!(preferences.strengths, vec!["code_generation".to_owned()]);
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/adaptive-classification-invalid.yaml");
    let error = settings(source, &[]).unwrap_err();
    assert!(!error.is_empty());
}
