#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::agent_search::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.agent_search
}

fn load_error(source: &str, variables: &[(&str, &str)]) -> ConfigError {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }.load(source).err().unwrap()
}

#[test]
fn defaults_when_the_file_has_no_agent_search_settings() {
    let loaded = settings("litellm_settings: {}", &[]);
    assert_eq!(loaded.embedding_model, None);
    assert_eq!(
        loaded.health_check_timeout_seconds.to_bits(),
        5.0_f64.to_bits()
    );
    assert_eq!(
        loaded.health_gather_timeout_seconds.to_bits(),
        30.0_f64.to_bits()
    );
    assert_eq!(Settings::default(), loaded);
}

#[test]
fn reads_the_embedding_model_from_litellm_settings() {
    let loaded = settings(
        "litellm_settings:\n  agent_search_embedding_model: text-embedding-3-small",
        &[],
    );
    assert_eq!(
        loaded.embedding_model.as_deref(),
        Some("text-embedding-3-small")
    );
}

#[test]
fn resolves_an_environment_reference_for_the_embedding_model() {
    let loaded = settings(
        "litellm_settings:\n  agent_search_embedding_model: os.environ/EMBED_MODEL",
        &[("EMBED_MODEL", "text-embedding-3-large")],
    );
    assert_eq!(
        loaded.embedding_model.as_deref(),
        Some("text-embedding-3-large")
    );
}

#[test]
fn an_unset_embedding_model_variable_leaves_search_unconfigured() {
    let loaded = settings(
        "litellm_settings:\n  agent_search_embedding_model: os.environ/EMBED_MODEL",
        &[],
    );
    assert_eq!(loaded.embedding_model, None);
}

#[test]
fn reads_both_health_timeouts_from_the_environment() {
    let loaded = settings(
        "litellm_settings: {}",
        &[
            ("LITELLM_AGENT_HEALTH_CHECK_TIMEOUT", "1.5"),
            ("LITELLM_AGENT_HEALTH_CHECK_GATHER_TIMEOUT", "9"),
        ],
    );
    assert_eq!(
        loaded.health_check_timeout_seconds.to_bits(),
        1.5_f64.to_bits()
    );
    assert_eq!(
        loaded.health_gather_timeout_seconds.to_bits(),
        9.0_f64.to_bits()
    );
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let loaded = settings(
        "litellm_settings: {}",
        &[
            ("LITELLM_AGENT_HEALTH_CHECK_TIMEOUT", ""),
            ("LITELLM_AGENT_HEALTH_CHECK_GATHER_TIMEOUT", ""),
        ],
    );
    assert_eq!(
        loaded.health_check_timeout_seconds.to_bits(),
        5.0_f64.to_bits()
    );
    assert_eq!(
        loaded.health_gather_timeout_seconds.to_bits(),
        30.0_f64.to_bits()
    );
}

#[rstest]
#[case::probe_timeout("LITELLM_AGENT_HEALTH_CHECK_TIMEOUT")]
#[case::gather_timeout("LITELLM_AGENT_HEALTH_CHECK_GATHER_TIMEOUT")]
fn a_non_numeric_timeout_is_a_startup_error_naming_the_variable(#[case] variable: &str) {
    let error = load_error("litellm_settings: {}", &[(variable, "soon")]);
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains(variable), "{error}");
}
