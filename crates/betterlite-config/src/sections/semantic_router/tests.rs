#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::semantic_router::{Deployment, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.semantic_router
}

fn deployment(source: &str) -> Option<Deployment> {
    settings(source, &[])
        .deployments
        .into_iter()
        .next()
        .flatten()
}

const FULL: &str = r#"model_list:
  - model_name: semantic-router
    litellm_params:
      model: auto_router/semantic
      auto_router_config_path: ./routes.json
      auto_router_config: '{"routes": []}'
      auto_router_default_model: fast
      auto_router_embedding_model: openai/text-embedding-3-small
      auto_router_max_input_chars: 2000
"#;

#[test]
fn every_declared_key_is_resolved() {
    let configured = deployment(FULL).expect("the deployment configures the auto router");
    assert_eq!(configured.config_path, Some(PathBuf::from("./routes.json")));
    assert_eq!(configured.config_json.as_deref(), Some(r#"{"routes": []}"#));
    assert_eq!(configured.default_model.as_deref(), Some("fast"));
    assert_eq!(
        configured.embedding_model.as_deref(),
        Some("openai/text-embedding-3-small")
    );
    assert_eq!(configured.max_input_chars, Some(2000));
}

#[test]
fn both_config_sources_are_kept_for_the_service_to_choose_between() {
    let configured = deployment(FULL).expect("both sources stay configured");
    assert!(configured.config_path.is_some());
    assert!(configured.config_json.is_some());
}

#[rstest]
#[case::empty_model_list("model_list: []")]
#[case::unrelated_deployment(
    "model_list:\n  - model_name: fast\n    litellm_params:\n      model: openai/gpt-4o-mini\n"
)]
#[case::null_keys(
    "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_config_path: null\n      auto_router_max_input_chars: null\n"
)]
fn a_deployment_without_router_keys_configures_nothing(#[case] source: &str) {
    assert_eq!(deployment(source), None);
}

#[test]
fn one_key_is_enough_to_configure_the_deployment() {
    let source = "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_default_model: fast\n";
    let configured = deployment(source).expect("a single key configures the deployment");
    assert_eq!(configured.default_model.as_deref(), Some("fast"));
    assert_eq!(configured.config_path, None);
    assert_eq!(configured.config_json, None);
    assert_eq!(configured.embedding_model, None);
    assert_eq!(configured.max_input_chars, None);
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
#[case::large(1_000_000)]
fn the_character_limit_is_kept_as_given(#[case] limit: i64) {
    let source = format!(
        "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_max_input_chars: {limit}\n"
    );
    let configured = deployment(&source).expect("the limit configures the deployment");
    assert_eq!(configured.max_input_chars, Some(limit));
}

#[test]
fn strings_resolve_environment_references() {
    let source = "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_config_path: os.environ/ROUTES\n      auto_router_default_model: os.environ/FALLBACK\n";
    let lookup = [("ROUTES", "./routes.json"), ("FALLBACK", "fast")];
    let loaded = settings(source, &lookup);
    let configured = loaded.deployments.into_iter().next().flatten().unwrap();
    assert_eq!(configured.config_path, Some(PathBuf::from("./routes.json")));
    assert_eq!(configured.default_model.as_deref(), Some("fast"));
}

#[test]
fn the_character_limit_resolves_an_environment_reference() {
    let source = "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_max_input_chars: os.environ/LIMIT\n";
    let loaded = settings(source, &[("LIMIT", "512")]);
    let configured = loaded.deployments.into_iter().next().flatten().unwrap();
    assert_eq!(configured.max_input_chars, Some(512));
}

#[test]
fn an_empty_environment_limit_counts_as_unset() {
    let source = "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_max_input_chars: os.environ/LIMIT\n";
    let loaded = settings(source, &[("LIMIT", "")]);
    let configured = loaded.deployments.into_iter().next().flatten().unwrap();
    assert_eq!(configured.max_input_chars, None);
}

#[test]
fn a_missing_environment_reference_is_a_load_error() {
    let source = "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_config_path: os.environ/ROUTES\n";
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load(source)
        .expect_err("a missing reference is a load error");
    assert_eq!(error.category(), "missing_environment");
    assert!(error.to_string().contains("ROUTES"));
}

#[rstest]
#[case::text("'many'")]
#[case::fraction("2000.5")]
#[case::boolean("true")]
fn a_limit_that_is_not_an_integer_is_a_load_error(#[case] value: &str) {
    let source = format!(
        "model_list:\n  - model_name: semantic\n    litellm_params:\n      auto_router_max_input_chars: {value}\n"
    );
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load(&source)
        .expect_err("a non-integer limit is a load error");
    assert!(
        error.to_string().contains("auto_router_max_input_chars"),
        "{error}"
    );
}

#[test]
fn deployments_stay_index_aligned_with_the_model_list() {
    let source = "model_list:\n  - model_name: fast\n    litellm_params:\n      model: openai/gpt-4o-mini\n  - model_name: semantic\n    litellm_params:\n      auto_router_default_model: fast\n";
    let loaded = settings(source, &[]);
    assert_eq!(loaded.deployments.len(), 2);
    assert_eq!(loaded.deployments.first(), Some(&None));
    assert!(loaded.deployments.last().is_some_and(Option::is_some));
}

#[test]
fn the_keys_are_no_longer_unsupported() {
    let loaded = settings(FULL, &[]);
    assert_eq!(loaded.deployments.len(), 1);
}
