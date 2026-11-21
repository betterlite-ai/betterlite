#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::complexity_placement::{Config, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.complexity_placement)
        .map_err(|error| error.to_string())
}

fn router(fields: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n{fields}"
    )
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
fn the_defaults_are_the_pinned_litellm_values() {
    assert_eq!(
        Config::default(),
        Config {
            return_raw_model_name: false,
            max_tokens_from_tier_model: true,
        }
    );
}

#[test]
fn an_empty_router_config_resolves_to_the_defaults() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config: {}\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(resolved.deployments, vec![Some(Config::default())]);
}

#[test]
fn declared_values_replace_the_defaults() {
    let source =
        router("        return_raw_model_name: true\n        max_tokens_from_tier_model: false\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Some(Config {
            return_raw_model_name: true,
            max_tokens_from_tier_model: false,
        })]
    );
}

#[test]
fn an_environment_flag_resolves_to_a_bool() {
    let source = router("        return_raw_model_name: os.environ/RAW_NAME\n");
    let resolved = settings(&source, &[("RAW_NAME", "yes")]).unwrap();
    assert!(
        resolved.deployments[0]
            .as_ref()
            .unwrap()
            .return_raw_model_name
    );
}

#[test]
fn an_empty_environment_flag_takes_the_default() {
    let source = router("        max_tokens_from_tier_model: os.environ/CEILING\n");
    let resolved = settings(&source, &[("CEILING", "")]).unwrap();
    assert!(
        resolved.deployments[0]
            .as_ref()
            .unwrap()
            .max_tokens_from_tier_model
    );
}

#[test]
fn a_missing_environment_flag_is_an_error() {
    let source = router("        return_raw_model_name: os.environ/RAW_NAME\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("RAW_NAME"), "{error}");
}

#[test]
fn a_text_flag_of_an_unknown_word_is_an_error() {
    let source = router("        return_raw_model_name: maybe\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("return_raw_model_name"), "{error}");
}

#[test]
fn a_flag_of_explicit_false_stays_off() {
    let source = router("        max_tokens_from_tier_model: \"off\"\n");
    let resolved = settings(&source, &[]).unwrap();
    assert!(
        !resolved.deployments[0]
            .as_ref()
            .unwrap()
            .max_tokens_from_tier_model
    );
}

#[test]
fn a_numeric_flag_is_rejected() {
    let source = router("        return_raw_model_name: 1\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("RawFlag"), "{error}");
}

#[test]
fn the_router_sibling_keys_do_not_become_unsupported() {
    let source = router(
        "        return_raw_model_name: true\n        tiers:\n          simple: mini\n        default_model: openai/gpt-4o\n",
    );
    let resolved = settings(&source, &[]).unwrap();
    assert!(
        resolved.deployments[0]
            .as_ref()
            .unwrap()
            .return_raw_model_name
    );
}

#[test]
fn an_extra_router_config_key_is_allowed() {
    // ComplexityRouterConfig is extra="allow", and the other section views claim these siblings
    let source = router("        placement_mystery: true\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(resolved.deployments, vec![Some(Config::default())]);
}
