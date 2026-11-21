#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::complexity_turn_overrides::{
    Config, DEFAULT_HOUSEKEEPING, DEFAULT_MATCH_THRESHOLD, DEFAULT_STALL_REPEAT,
    DEFAULT_STALL_WINDOW, KeywordMatching, KeywordTierRule, Settings,
};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.complexity_turn_overrides)
        .map_err(|error| error.to_string())
}

fn router(fields: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n{fields}"
    )
}

fn configured(fields: &str, variables: &[(&str, &str)]) -> Result<Config, String> {
    let source = router(fields);
    let resolved = settings(&source, variables)?;
    Ok(resolved.deployments[0].clone().unwrap_or_default())
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
            keyword_tier_rules: None,
            stall_escalation_enabled: false,
            stall_escalation_window: DEFAULT_STALL_WINDOW,
            stall_repeat_threshold: DEFAULT_STALL_REPEAT,
            keyword_matching: KeywordMatching::Lexical,
            embedding_model: None,
            match_threshold: DEFAULT_MATCH_THRESHOLD,
            housekeeping_cheapest: DEFAULT_HOUSEKEEPING,
        }
    );
}

#[test]
fn an_empty_router_config_resolves_to_the_defaults() {
    let source = router("        {}\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(resolved.deployments, vec![Some(Config::default())]);
}

#[test]
fn declared_values_replace_the_defaults() {
    let fields = concat!(
        "        keyword_tier_rules:\n",
        "          - keywords: [\" invoice \", \"\"]\n",
        "            tier: \" COMPLEX \"\n",
        "          - keywords: [refund]\n",
        "            tier: SIMPLE\n",
        "        stall_escalation_enabled: true\n",
        "        stall_escalation_window: 4\n",
        "        stall_escalation_repeat_threshold: 2\n",
        "        semantic_keyword_matching: true\n",
        "        embedding_model: openai/text-embedding-3-small\n",
        "        match_threshold: 0.7\n",
        "        route_housekeeping_to_cheapest_tier: false\n"
    );
    let found = configured(fields, &[]).unwrap();
    assert_eq!(
        found.keyword_tier_rules,
        Some(vec![
            KeywordTierRule {
                keywords: vec!["invoice".to_owned()],
                tier: "COMPLEX".to_owned(),
            },
            KeywordTierRule {
                keywords: vec!["refund".to_owned()],
                tier: "SIMPLE".to_owned(),
            },
        ])
    );
    assert!(found.stall_escalation_enabled);
    assert_eq!(found.stall_escalation_window, 4);
    assert_eq!(found.stall_repeat_threshold, 2);
    assert_eq!(found.keyword_matching, KeywordMatching::Semantic);
    assert_eq!(
        found.embedding_model.as_deref(),
        Some("openai/text-embedding-3-small")
    );
    assert_eq!(found.match_threshold, 0.7);
    assert!(!found.housekeeping_cheapest);
}

#[test]
fn environment_values_resolve_once_in_the_config() {
    let fields = concat!(
        "        stall_escalation_enabled: os.environ/STALL\n",
        "        stall_escalation_window: os.environ/WINDOW\n",
        "        match_threshold: os.environ/THRESHOLD\n",
        "        embedding_model: os.environ/MODEL\n",
        "        route_housekeeping_to_cheapest_tier: os.environ/HOUSEKEEPING\n"
    );
    let found = configured(
        fields,
        &[
            ("STALL", "yes"),
            ("WINDOW", "8"),
            ("THRESHOLD", "0.25"),
            ("MODEL", ""),
            ("HOUSEKEEPING", "no"),
        ],
    )
    .unwrap();
    assert!(found.stall_escalation_enabled);
    assert_eq!(found.stall_escalation_window, 8);
    assert_eq!(found.match_threshold, 0.25);
    assert_eq!(found.embedding_model, None);
    assert!(!found.housekeeping_cheapest);
}

#[test]
fn a_blank_keyword_list_is_rejected() {
    let fields =
        "        keyword_tier_rules:\n          - keywords: [\"  \"]\n            tier: SIMPLE\n";
    let error = configured(fields, &[]).unwrap_err();
    assert!(error.contains("keywords"), "{error}");
}

#[test]
fn a_rule_without_a_tier_is_rejected() {
    let fields = "        keyword_tier_rules:\n          - keywords: [invoice]\n";
    let error = configured(fields, &[]).unwrap_err();
    assert!(error.contains("tier"), "{error}");
}

#[test]
fn a_zero_stall_window_is_rejected() {
    let fields = "        stall_escalation_window: 0\n";
    let error = configured(fields, &[]).unwrap_err();
    assert!(error.contains("stall_escalation_window"), "{error}");
}

#[test]
fn a_repeat_threshold_below_two_is_rejected() {
    let fields = "        stall_escalation_repeat_threshold: 1\n";
    let error = configured(fields, &[]).unwrap_err();
    assert!(
        error.contains("stall_escalation_repeat_threshold"),
        "{error}"
    );
}

#[rstest]
#[case("        match_threshold: 1.5\n")]
#[case("        match_threshold: -0.1\n")]
fn a_match_threshold_outside_the_unit_interval_is_rejected(#[case] fields: &str) {
    let error = configured(fields, &[]).unwrap_err();
    assert!(error.contains("match_threshold"), "{error}");
}

#[test]
fn a_quoted_number_is_rejected_as_the_wrong_type() {
    let fields = "        match_threshold: \"0.5\"\n";
    let error = configured(fields, &[]).unwrap_err();
    assert!(error.contains("match_threshold"), "{error}");
}

#[test]
fn a_repeat_threshold_above_the_window_is_left_to_the_cross_field_checks() {
    let fields =
        "        stall_escalation_window: 2\n        stall_escalation_repeat_threshold: 3\n";
    let found = configured(fields, &[]).unwrap();
    assert_eq!(found.stall_escalation_window, 2);
    assert_eq!(found.stall_repeat_threshold, 3);
}
