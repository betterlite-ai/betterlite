#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::router_compaction::{
    ContextCompaction, ContextCompactionConfig, DEFAULT_MAX_TOKENS, DEFAULT_TIMEOUT_SECONDS,
    DEFAULT_TRIGGER_RATIO, Settings,
};

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.router_compaction)
}

fn deployment(router_config: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n{router_config}"
    )
}

fn with_value(value: &str) -> String {
    deployment(&format!(
        "      complexity_router_config:\n        context_compaction: {value}\n"
    ))
}

fn object(keys: &str) -> String {
    with_value(&format!("{{{keys}}}"))
}

fn second_deployment(router_config: &str) -> String {
    format!(
        "model_list:\n  - model_name: plain\n    litellm_params:\n      model: openai/gpt-4o\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o-mini\n{router_config}"
    )
}

fn only(source: &str) -> Result<Option<ContextCompaction>, String> {
    let mut deployments = load_with(source, &[])
        .map_err(|error| error.to_string())?
        .deployments;
    if deployments.len() != 1 {
        return Err(format!(
            "one deployment expected, got {}",
            deployments.len()
        ));
    }
    Ok(deployments.pop().unwrap_or_default())
}

fn configured(source: &str) -> ContextCompaction {
    only(source)
        .expect("the fixture config loads")
        .expect("the deployment configures compaction")
}

fn defaults() -> ContextCompactionConfig {
    ContextCompactionConfig {
        model: None,
        trigger_ratio: DEFAULT_TRIGGER_RATIO,
        max_tokens: DEFAULT_MAX_TOKENS,
        timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
    }
}

#[test]
fn a_deployment_without_router_configuration_keeps_none() {
    assert_eq!(only(&deployment("")).unwrap(), None);
}

#[test]
fn a_missing_context_compaction_compacts_with_the_defaults() {
    let source = deployment("      complexity_router_config: {}\n");
    assert_eq!(configured(&source), ContextCompaction::On(defaults()));
}

#[test]
fn settings_stay_aligned_with_model_list() {
    let source =
        second_deployment("      complexity_router_config:\n        context_compaction: false\n");
    assert_eq!(
        load_with(&source, &[])
            .expect("the fixture config loads")
            .deployments,
        [None, Some(ContextCompaction::Off)]
    );
}

#[test]
fn an_error_in_a_later_deployment_names_its_index() {
    let source = second_deployment(
        "      complexity_router_config:\n        context_compaction: {trigger_ratio: 1}\n",
    );
    let error = load_with(&source, &[]).unwrap_err().to_string();
    assert!(
        error.contains(
            "model_list.1.litellm_params.complexity_router_config.context_compaction.trigger_ratio"
        ),
        "{error}"
    );
}

#[test]
fn a_false_value_turns_compaction_off() {
    assert_eq!(configured(&with_value("false")), ContextCompaction::Off);
}

#[test]
fn a_null_value_turns_compaction_off() {
    assert_eq!(configured(&with_value("null")), ContextCompaction::Off);
}

#[test]
fn a_true_value_is_refused_and_names_its_path() {
    let error = only(&with_value("true")).unwrap_err();
    assert!(
        error.contains("model_list.0.litellm_params.complexity_router_config.context_compaction"),
        "{error}"
    );
}

#[test]
fn every_key_of_a_full_object_resolves() {
    let source =
        object("model: gpt-4o-mini, trigger_ratio: 0.5, max_tokens: 2048, timeout_seconds: 30");
    assert_eq!(
        configured(&source),
        ContextCompaction::On(ContextCompactionConfig {
            model: Some("gpt-4o-mini".to_owned()),
            trigger_ratio: 0.5,
            max_tokens: 2048,
            timeout_seconds: 30.0,
        })
    );
}

#[rstest]
#[case::just_inside_zero(0.001_f64)]
#[case::just_inside_one(0.999_f64)]
fn a_trigger_ratio_inside_the_open_range_loads(#[case] ratio: f64) {
    let source = object(&format!("trigger_ratio: {ratio}"));
    assert_eq!(
        configured(&source),
        ContextCompaction::On(ContextCompactionConfig {
            trigger_ratio: ratio,
            ..defaults()
        })
    );
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::one(1.0_f64)]
#[case::negative(-0.5_f64)]
#[case::above_one(1.5_f64)]
fn a_trigger_ratio_outside_the_open_range_names_its_path(#[case] ratio: f64) {
    let error = only(&object(&format!("trigger_ratio: {ratio}"))).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.context_compaction.trigger_ratio"
        ),
        "{error}"
    );
}

#[test]
fn the_lowest_max_tokens_loads() {
    assert_eq!(
        configured(&object("max_tokens: 512")),
        ContextCompaction::On(ContextCompactionConfig {
            max_tokens: 512,
            ..defaults()
        })
    );
}

#[test]
fn a_max_tokens_below_the_floor_names_its_path() {
    let error = only(&object("max_tokens: 511")).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.context_compaction.max_tokens"
        ),
        "{error}"
    );
}

#[test]
fn a_positive_timeout_loads() {
    assert_eq!(
        configured(&object("timeout_seconds: 0.001")),
        ContextCompaction::On(ContextCompactionConfig {
            timeout_seconds: 0.001,
            ..defaults()
        })
    );
}

#[test]
fn a_non_positive_timeout_names_its_path() {
    let zero = only(&object("timeout_seconds: 0")).unwrap_err();
    assert_eq!(
        zero,
        "invalid scalar at model_list.0.litellm_params.complexity_router_config.context_compaction.timeout_seconds"
    );
    let negative = only(&object("timeout_seconds: -1")).unwrap_err();
    assert_eq!(
        negative,
        "invalid scalar at model_list.0.litellm_params.complexity_router_config.context_compaction.timeout_seconds"
    );
}

#[test]
fn an_empty_model_names_its_path() {
    let error = only(&object("model: \"\"")).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.context_compaction.model"
        ),
        "{error}"
    );
}

#[test]
fn an_unknown_key_names_its_path() {
    let error = only(&object("model: gpt-4o, colour: red")).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.context_compaction.colour"
        ),
        "{error}"
    );
}

#[test]
fn environment_references_resolve() {
    let source = object(
        "model: os.environ/MODEL, trigger_ratio: os.environ/RATIO, max_tokens: os.environ/TOKENS, timeout_seconds: os.environ/TIMEOUT",
    );
    let settings = load_with(
        &source,
        &[
            ("MODEL", "gpt-4o-mini"),
            ("RATIO", "0.25"),
            ("TOKENS", "1024"),
            ("TIMEOUT", "5"),
        ],
    )
    .expect("the fixture config loads");
    assert_eq!(
        settings.deployments,
        [Some(ContextCompaction::On(ContextCompactionConfig {
            model: Some("gpt-4o-mini".to_owned()),
            trigger_ratio: 0.25,
            max_tokens: 1024,
            timeout_seconds: 5.0,
        }))]
    );
}

#[test]
fn an_empty_environment_model_counts_as_unset() {
    let settings = load_with(&object("model: os.environ/MODEL"), &[("MODEL", "")])
        .expect("the fixture config loads");
    assert_eq!(
        settings.deployments,
        [Some(ContextCompaction::On(defaults()))]
    );
}

#[test]
fn a_missing_environment_variable_names_its_path() {
    let error = only(&object("model: os.environ/MODEL")).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.context_compaction.model"
        ),
        "{error}"
    );
}
