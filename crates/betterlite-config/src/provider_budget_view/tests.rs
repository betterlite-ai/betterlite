#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{ProviderBudgetConfig, load_provider_budget_view};
use crate::loader::ConfigError;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

fn providers(settings: super::Settings) -> indexmap::IndexMap<String, ProviderBudgetConfig> {
    settings
        .provider_budget_config
        .expect("the config keeps its provider map")
}

#[test]
fn loads_provider_budget_view() {
    let source = "router_settings:\n  provider_budget_config:\n    openai:\n      budget_limit: 100\n      time_period: 1d\n    azure:\n      budget_limit: 12.5\n      time_period: 30d\n";
    let loaded =
        providers(load_provider_budget_view(source, &BTreeMap::<String, String>::new()).unwrap());
    assert_eq!(loaded.len(), 2);
    assert_eq!(
        loaded["openai"],
        ProviderBudgetConfig {
            max_budget: Some(decimal("100")),
            budget_duration: Some("1d".to_owned()),
        }
    );
    assert_eq!(
        loaded["azure"],
        ProviderBudgetConfig {
            max_budget: Some(decimal("12.5")),
            budget_duration: Some("30d".to_owned()),
        }
    );
}

#[rstest]
#[case::absent("litellm_settings:\n  max_budget: 5\n", None)]
#[case::explicit_null("router_settings:\n  provider_budget_config: null\n", None)]
#[case::empty_map("router_settings:\n  provider_budget_config: {}\n", Some(0))]
fn provider_budget_config_is_null_unless_the_map_exists(
    #[case] source: &str,
    #[case] entries: Option<usize>,
) {
    let settings = load_provider_budget_view(source, &BTreeMap::<String, String>::new()).unwrap();
    assert_eq!(
        settings
            .provider_budget_config
            .as_ref()
            .map(indexmap::IndexMap::len),
        entries
    );
}

#[rstest]
#[case::budget_limit_null("openai:\n      budget_limit: null")]
#[case::time_period_null("openai:\n      time_period: null")]
#[case::empty_map("openai: {}")]
fn a_provider_entry_keeps_its_fields_optional(#[case] entry: &str) {
    let source = format!("router_settings:\n  provider_budget_config:\n    {entry}\n");
    let loaded =
        providers(load_provider_budget_view(&source, &BTreeMap::<String, String>::new()).unwrap());
    assert_eq!(loaded["openai"], ProviderBudgetConfig::default());
}

#[rstest]
#[case::max_budget("max_budget")]
#[case::budget_duration("budget_duration")]
#[case::tpm_limit("tpm_limit")]
#[case::rpm_limit("rpm_limit")]
fn ignores_names_the_provider_tier_never_reads(#[case] name: &str) {
    let source =
        format!("router_settings:\n  provider_budget_config:\n    openai:\n      {name}: 7\n");
    let loaded =
        providers(load_provider_budget_view(&source, &BTreeMap::<String, String>::new()).unwrap());
    assert_eq!(loaded["openai"], ProviderBudgetConfig::default());
}

#[test]
fn resolves_environment_references_into_both_fields() {
    let source = "router_settings:\n  provider_budget_config:\n    openai:\n      budget_limit: os.environ/OPENAI_LIMIT\n      time_period: os.environ/OPENAI_PERIOD\n";
    let lookup = environment(&[("OPENAI_LIMIT", "40"), ("OPENAI_PERIOD", "7d")]);
    let loaded = providers(load_provider_budget_view(source, &lookup).unwrap());
    assert_eq!(
        loaded["openai"],
        ProviderBudgetConfig {
            max_budget: Some(decimal("40")),
            budget_duration: Some("7d".to_owned()),
        }
    );
}

#[test]
fn an_empty_environment_reference_leaves_its_field_unset() {
    let source = "router_settings:\n  provider_budget_config:\n    openai:\n      budget_limit: os.environ/OPENAI_LIMIT\n      time_period: os.environ/OPENAI_PERIOD\n";
    let lookup = environment(&[("OPENAI_LIMIT", ""), ("OPENAI_PERIOD", "")]);
    let loaded = providers(load_provider_budget_view(source, &lookup).unwrap());
    assert_eq!(loaded["openai"], ProviderBudgetConfig::default());
}

#[test]
fn a_missing_environment_reference_fails_the_load() {
    let source = "router_settings:\n  provider_budget_config:\n    openai:\n      budget_limit: os.environ/OPENAI_LIMIT\n";
    let error = load_provider_budget_view(source, &BTreeMap::<String, String>::new()).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "OPENAI_LIMIT"
                && path == "router_settings.provider_budget_config.openai.budget_limit"
    ));
}

#[test]
fn keeps_an_empty_time_period_written_into_the_file() {
    let source =
        "router_settings:\n  provider_budget_config:\n    openai:\n      time_period: \"\"\n";
    let loaded =
        providers(load_provider_budget_view(source, &BTreeMap::<String, String>::new()).unwrap());
    assert_eq!(loaded["openai"].budget_duration.as_deref(), Some(""));
}

#[test]
fn rejects_a_null_provider_entry() {
    let source = "router_settings:\n  provider_budget_config:\n    openai:\n";
    let error = load_provider_budget_view(source, &BTreeMap::<String, String>::new()).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. }
            if path == "router_settings.provider_budget_config.openai"
    ));
}

#[rstest]
fn replays_the_provider_budget_view_corpus(
    #[files("fuzz/corpus/config_yaml/provider-budget-view-*")] path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    drop(load_provider_budget_view(
        &source,
        &BTreeMap::<String, String>::new(),
    ));
}
