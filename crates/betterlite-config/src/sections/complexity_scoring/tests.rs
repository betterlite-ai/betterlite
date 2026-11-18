#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::complexity_scoring::{Config, Settings, TierModelValue};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.complexity_scoring)
        .map_err(|error| error.to_string())
}

// one deployment whose complexity router config holds the given keys, already indented under it
fn deployment(keys: &str) -> String {
    format!(
        "model_list:\n  - model_name: primary\n    litellm_params:\n      complexity_router_config:\n{keys}"
    )
}

fn only(keys: &str, variables: &[(&str, &str)]) -> Result<Option<Config>, String> {
    let mut deployments = settings(&deployment(keys), variables)?.deployments;
    let count = deployments.len();
    if count != 1 {
        return Err(format!("one deployment expected, got {count}"));
    }
    Ok(deployments.pop().flatten())
}

fn weights(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs
        .iter()
        .map(|(name, weight)| ((*name).to_owned(), *weight))
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn single(value: &str) -> TierModelValue {
    TierModelValue::Single(value.to_owned())
}

#[test]
fn a_config_without_a_model_list_has_no_deployments() {
    assert_eq!(settings("{}", &[]).unwrap(), Settings::default());
    assert_eq!(
        settings("model_list: []", &[]).unwrap(),
        Settings::default()
    );
}

#[test]
fn a_deployment_without_the_router_config_carries_none() {
    let source = "model_list:\n  - model_name: primary\n    litellm_params:\n      model: gpt-4o\n";
    assert_eq!(settings(source, &[]).unwrap().deployments, [None]);
}

const BASELINE_KEYS: &str = "        tier_models:
          SIMPLE: small-model
          MEDIUM: [pool-a, pool-b]
        complexity_thresholds:
          simple: 10
          complex: 500
        dimension_weights:
          codePresence: 0.5
        tier_boundaries:
          simple_medium: 0.2
        reasoning_override_min_score: 0.4
        code_keywords: [alpha, beta]
        reasoning_keywords: [gamma]
        technical_keywords: [delta]
        custom_technical_keywords: [epsilon]
        simple_keywords: [zeta]
        default_model: fallback-model
";

#[test]
fn every_baseline_key_is_read() {
    let mut tier_models = BTreeMap::new();
    tier_models.insert("SIMPLE".to_owned(), single("small-model"));
    tier_models.insert(
        "MEDIUM".to_owned(),
        TierModelValue::Pool(strings(&["pool-a", "pool-b"])),
    );
    let mut thresholds = BTreeMap::new();
    thresholds.insert("simple".to_owned(), 10);
    thresholds.insert("complex".to_owned(), 500);
    let expected = Config {
        tier_models: Some(tier_models),
        complexity_thresholds: Some(thresholds),
        dimension_weights: Some(weights(&[("codePresence", 0.5_f64)])),
        tier_boundaries: Some(weights(&[("simple_medium", 0.2_f64)])),
        reasoning_override_min_score: Some(0.4_f64),
        code_keywords: Some(strings(&["alpha", "beta"])),
        reasoning_keywords: Some(strings(&["gamma"])),
        technical_keywords: Some(strings(&["delta"])),
        custom_technical_keywords: Some(strings(&["epsilon"])),
        simple_keywords: Some(strings(&["zeta"])),
        default_model: Some("fallback-model".to_owned()),
    };
    assert_eq!(only(BASELINE_KEYS, &[]).unwrap(), Some(expected));
}

#[test]
fn the_litellm_spellings_are_aliases_for_the_same_keys() {
    let litellm = "        tiers:
          SIMPLE: small-model
        token_thresholds:
          simple: 12
";
    let betterlite = "        tier_models:
          SIMPLE: small-model
        complexity_thresholds:
          simple: 12
";
    assert_eq!(only(litellm, &[]).unwrap(), only(betterlite, &[]).unwrap());
}

#[rstest]
#[case::tiers("tiers", "tier_models")]
#[case::thresholds("token_thresholds", "complexity_thresholds")]
fn both_spellings_of_one_key_are_rejected(#[case] first: &str, #[case] second: &str) {
    let keys = format!("        {first}: {{}}\n        {second}: {{}}\n");
    let error = settings(&deployment(&keys), &[]).unwrap_err();
    assert!(error.contains("duplicate field"), "got {error}");
}

#[test]
fn nulls_read_as_absent() {
    let keys = "        tier_models: null
        complexity_thresholds: null
        dimension_weights: null
        tier_boundaries: null
        reasoning_override_min_score: null
        code_keywords: null
        reasoning_keywords: null
        technical_keywords: null
        custom_technical_keywords: null
        simple_keywords: null
        default_model: null
";
    assert_eq!(only(keys, &[]).unwrap(), Some(Config::default()));
}

#[test]
fn an_unknown_router_key_is_ignored_by_this_section() {
    let keys = "        default_model: fallback-model
        adaptive_routing:
          enabled: true
";
    let configured = only(keys, &[]).unwrap().unwrap();
    assert_eq!(configured.default_model, Some("fallback-model".to_owned()));
    assert_eq!(configured.tier_models, None);
}

#[test]
fn a_tier_matrix_reads_a_bare_model_and_a_pool() {
    let keys = "        tier_models:
          SIMPLE: single
          COMPLEX: [pool-first, pool-second]
";
    let configured = only(keys, &[]).unwrap().unwrap().tier_models.unwrap();
    assert_eq!(configured.get("SIMPLE"), Some(&single("single")));
    assert_eq!(
        configured.get("COMPLEX"),
        Some(&TierModelValue::Pool(strings(&[
            "pool-first",
            "pool-second"
        ])))
    );
}

#[test]
fn a_parameterized_tier_entry_is_rejected() {
    let keys = "        tier_models:\n          SIMPLE:\n            model: nested\n";
    let error = settings(&deployment(keys), &[]).unwrap_err();
    assert!(
        error.contains("a model name or a list of model names"),
        "got {error}"
    );
}

#[test]
fn a_tier_model_environment_reference_is_resolved() {
    let keys = "        tier_models:\n          SIMPLE: os.environ/TIER_MODEL\n";
    let configured = only(keys, &[("TIER_MODEL", "small-model")]).unwrap();
    let tier_models = configured.unwrap().tier_models.unwrap();
    assert_eq!(tier_models.get("SIMPLE"), Some(&single("small-model")));
}

#[test]
fn a_default_model_environment_reference_is_resolved() {
    let keys = "        default_model: os.environ/FALLBACK_MODEL\n";
    let configured = only(keys, &[("FALLBACK_MODEL", "fallback-model")]).unwrap();
    assert_eq!(
        configured.unwrap().default_model,
        Some("fallback-model".to_owned())
    );
}

#[test]
fn a_pool_item_stays_literal() {
    let keys = "        tier_models:\n          SIMPLE: [os.environ/POOL_MODEL, plain-model]\n";
    let configured = only(keys, &[("POOL_MODEL", "resolved-model")]).unwrap();
    let tier_models = configured.unwrap().tier_models.unwrap();
    assert_eq!(
        tier_models.get("SIMPLE"),
        Some(&TierModelValue::Pool(strings(&[
            "os.environ/POOL_MODEL",
            "plain-model"
        ])))
    );
}

#[test]
fn a_threshold_environment_reference_is_resolved() {
    let keys = "        complexity_thresholds:\n          simple: os.environ/SIMPLE_TOKENS\n";
    let configured = only(keys, &[("SIMPLE_TOKENS", "9")]).unwrap();
    let thresholds = configured.unwrap().complexity_thresholds.unwrap();
    assert_eq!(thresholds.get("simple"), Some(&9));
}

#[test]
fn a_weight_environment_reference_is_resolved() {
    let keys = "        dimension_weights:\n          codePresence: os.environ/CODE_WEIGHT\n";
    let configured = only(keys, &[("CODE_WEIGHT", "0.5")]).unwrap();
    let weights = configured.unwrap().dimension_weights.unwrap();
    assert_eq!(weights.get("codePresence"), Some(&0.5_f64));
}

#[test]
fn a_boundary_environment_reference_is_resolved() {
    let keys = "        tier_boundaries:\n          simple_medium: os.environ/SIMPLE_MEDIUM\n";
    let configured = only(keys, &[("SIMPLE_MEDIUM", "0.2")]).unwrap();
    let boundaries = configured.unwrap().tier_boundaries.unwrap();
    assert_eq!(boundaries.get("simple_medium"), Some(&0.2_f64));
}

#[test]
fn a_floor_environment_reference_is_resolved() {
    let keys = "        reasoning_override_min_score: os.environ/OVERRIDE_FLOOR\n";
    let configured = only(keys, &[("OVERRIDE_FLOOR", "0.4")]).unwrap();
    assert_eq!(
        configured.unwrap().reasoning_override_min_score,
        Some(0.4_f64)
    );
}

#[rstest]
#[case::tier_model("tier_models:\n          SIMPLE: os.environ/UNSET_TIER")]
#[case::weight("dimension_weights:\n          codePresence: os.environ/UNSET_WEIGHT")]
#[case::floor("reasoning_override_min_score: os.environ/UNSET_FLOOR")]
#[case::default_model("default_model: os.environ/UNSET_DEFAULT")]
fn a_missing_environment_names_the_variable(#[case] body: &str) {
    let keys = format!("        {body}\n");
    let error = settings(&deployment(&keys), &[]).unwrap_err();
    assert!(error.contains("is not set"), "got {error}");
    assert!(error.contains("UNSET_"), "got {error}");
}

#[rstest]
#[case::weight(
    "dimension_weights:\n          codePresence: os.environ/BAD_WEIGHT",
    "BAD_WEIGHT"
)]
#[case::floor("reasoning_override_min_score: os.environ/BAD_FLOOR", "BAD_FLOOR")]
fn an_invalid_environment_value_names_the_variable(#[case] body: &str, #[case] variable: &str) {
    let keys = format!("        {body}\n");
    let error = settings(&deployment(&keys), &[(variable, "soon")]).unwrap_err();
    assert!(error.contains(variable), "got {error}");
}

#[test]
fn an_empty_environment_value_leaves_the_key_unset() {
    let keys = "        dimension_weights:\n          codePresence: os.environ/CODE_WEIGHT\n";
    let configured = only(keys, &[("CODE_WEIGHT", "")]).unwrap().unwrap();
    assert_eq!(configured.dimension_weights, Some(BTreeMap::new()));
}

#[rstest]
#[case::threshold("complexity_thresholds:\n          simple: \"15\"")]
#[case::weight("dimension_weights:\n          codePresence: \"0.3\"")]
#[case::boundary("tier_boundaries:\n          simple_medium: \"0.15\"")]
#[case::floor("reasoning_override_min_score: \"0.2\"")]
fn a_quoted_number_is_rejected(#[case] body: &str) {
    let keys = format!("        {body}\n");
    let error = settings(&deployment(&keys), &[]).unwrap_err();
    assert!(error.contains("invalid scalar"), "got {error}");
}

#[rstest]
#[case::fractional("1.5")]
#[case::overflowing("99999999999999999999999")]
fn a_threshold_that_is_not_an_integer_is_rejected(#[case] value: &str) {
    let keys = format!("        complexity_thresholds:\n          simple: {value}\n");
    let error = settings(&deployment(&keys), &[]).unwrap_err();
    assert!(error.contains("invalid integer"), "got {error}");
    assert!(
        error.contains("complexity_thresholds.simple"),
        "got {error}"
    );
}

#[test]
fn deployments_stay_aligned_with_the_model_list() {
    let source = "model_list:
  - model_name: first
    litellm_params:
      model: gpt-4o
  - model_name: second
    litellm_params:
      complexity_router_config:
        default_model: fallback
  - model_name: third
    litellm_params:
      model: gpt-4o-mini
";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(loaded.deployments.len(), 3);
    assert_eq!(loaded.deployments[0], None);
    let second = loaded.deployments[1].clone().unwrap();
    assert_eq!(second.default_model, Some("fallback".to_owned()));
    assert_eq!(loaded.deployments[2], None);
}

#[test]
fn the_path_of_a_bad_value_names_the_deployment_and_key() {
    let keys = "        dimension_weights:\n          codePresence: os.environ/MISSING_WEIGHT\n";
    let error = settings(&deployment(keys), &[]).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.dimension_weights.codePresence"
        ),
        "got {error}"
    );
}
