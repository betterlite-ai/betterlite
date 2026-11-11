#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::fallback_plans::Settings;
use crate::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.fallback_plans)
        .map_err(|error| error.to_string())
}

fn group(name: &str, targets: &[&str]) -> FallbackChainEntry {
    FallbackChainEntry::Group(IndexMap::from([(
        name.to_owned(),
        targets
            .iter()
            .map(|target| FallbackTarget::Model((*target).to_owned()))
            .collect(),
    )]))
}

#[test]
fn absent_keys_take_the_declaration_defaults() {
    let loaded = settings("{}", &[]).unwrap();
    assert_eq!(loaded.fallbacks, Vec::new());
    assert_eq!(loaded.context_window_fallbacks, Vec::new());
    assert_eq!(loaded.content_policy_fallbacks, Vec::new());
    assert_eq!(loaded.max_fallbacks, 5);
}

#[test]
fn router_settings_chains_are_typed() {
    let source = "router_settings:\n  fallbacks: [{gpt-4: [claude]}]\n  context_window_fallbacks: [{gpt-4: [long]}]\n  content_policy_fallbacks: [{gpt-4: [safe]}]\n";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(loaded.fallbacks, vec![group("gpt-4", &["claude"])]);
    assert_eq!(
        loaded.context_window_fallbacks,
        vec![group("gpt-4", &["long"])]
    );
    assert_eq!(
        loaded.content_policy_fallbacks,
        vec![group("gpt-4", &["safe"])]
    );
}

#[test]
fn litellm_settings_chains_fill_in_when_the_router_names_none() {
    let source = "litellm_settings:\n  fallbacks: [{gpt-4: [claude]}]\n";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(loaded.fallbacks, vec![group("gpt-4", &["claude"])]);
}

#[test]
fn an_empty_router_chain_is_used_as_given() {
    let source =
        "router_settings:\n  fallbacks: []\nlitellm_settings:\n  fallbacks: [{gpt-4: [claude]}]\n";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(loaded.fallbacks, Vec::new());
}

#[test]
fn default_fallbacks_become_the_generic_chain() {
    let source =
        "router_settings:\n  fallbacks: [{gpt-4: [claude]}]\n  default_fallbacks: [backup]\n";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(
        loaded.fallbacks,
        vec![group("gpt-4", &["claude"]), group("*", &["backup"])]
    );
}

#[test]
fn parameterized_targets_keep_their_overrides() {
    let source = "litellm_settings:\n  fallbacks: [{gpt-4: [{model: claude, messages: []}]}]\n";
    let loaded = settings(source, &[]).unwrap();
    let FallbackChainEntry::Group(chain) = &loaded.fallbacks[0] else {
        panic!("the entry is not a group");
    };
    let FallbackTarget::Parameters(target) = &chain["gpt-4"][0] else {
        panic!("the target is not parameterized");
    };
    assert_eq!(target.model, "claude");
    assert_eq!(target.parameters.keys().collect::<Vec<_>>(), ["messages"]);
}

#[test]
fn router_settings_parameterized_targets_load() {
    let source = "router_settings:\n  fallbacks: [{gpt-4: [{model: claude, messages: []}]}]\n";
    let loaded = settings(source, &[]).unwrap();
    let FallbackChainEntry::Group(chain) = &loaded.fallbacks[0] else {
        panic!("the entry is not a group");
    };
    let FallbackTarget::Parameters(target) = &chain["gpt-4"][0] else {
        panic!("the target is not parameterized");
    };
    assert_eq!(target.model, "claude");
    assert_eq!(target.parameters.keys().collect::<Vec<_>>(), ["messages"]);
}

#[rstest]
#[case::bare_string(
    "litellm_settings:\n  fallbacks: [claude]\n",
    "litellm_settings.fallbacks[0]"
)]
#[case::two_keys(
    "router_settings:\n  fallbacks: [{gpt-4: [claude], gpt-3.5: [claude]}]\n",
    "router_settings.fallbacks[0]"
)]
#[case::second_entry(
    "litellm_settings:\n  fallbacks: [{gpt-4: [claude]}, claude]\n",
    "litellm_settings.fallbacks[1]"
)]
#[case::content_policy(
    "litellm_settings:\n  content_policy_fallbacks: [claude]\n",
    "litellm_settings.content_policy_fallbacks[0]"
)]
fn rejects_chain_entries_that_name_no_single_group(#[case] source: &str, #[case] named: &str) {
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains(named), "{error}");
}

#[test]
fn context_window_chains_keep_the_entries_litellm_never_validates() {
    let source =
        "router_settings:\n  context_window_fallbacks: [long-ctx, {gpt-4: [a], gpt-3.5: [b]}]\n";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(
        loaded.context_window_fallbacks,
        vec![
            FallbackChainEntry::Target(FallbackTarget::Model(String::from("long-ctx"))),
            FallbackChainEntry::Group(IndexMap::from([
                (
                    String::from("gpt-4"),
                    vec![FallbackTarget::Model(String::from("a"))]
                ),
                (
                    String::from("gpt-3.5"),
                    vec![FallbackTarget::Model(String::from("b"))]
                ),
            ])),
        ]
    );
}

#[rstest]
#[case::environment_only(None, Some("3"), 3)]
#[case::configured_wins(Some(2), Some("7"), 2)]
#[case::unset_environment(None, None, 5)]
#[case::empty_environment(None, Some(""), 5)]
fn max_fallbacks_reads_the_setting_then_the_environment(
    #[case] configured: Option<i64>,
    #[case] environment: Option<&str>,
    #[case] expected: i64,
) {
    let setting = configured.map_or_else(
        || String::from("  {}"),
        |value| format!("  {{max_fallbacks: {value}}}"),
    );
    let source = format!("router_settings:\n{setting}");
    let variables: Vec<(&str, &str)> = environment
        .map(|value| vec![("ROUTER_MAX_FALLBACKS", value)])
        .unwrap_or_default();
    assert_eq!(
        settings(&source, &variables).unwrap().max_fallbacks,
        expected
    );
}

#[test]
fn max_fallbacks_rejects_an_environment_value_that_is_not_a_number() {
    let error = settings("{}", &[("ROUTER_MAX_FALLBACKS", "many")]).unwrap_err();
    assert!(error.contains("ROUTER_MAX_FALLBACKS"), "{error}");
}

#[test]
fn litellm_settings_max_fallbacks_are_used_when_the_router_names_none() {
    let loaded = settings("litellm_settings:\n  max_fallbacks: 2\n", &[]).unwrap();
    assert_eq!(loaded.max_fallbacks, 2);
}
