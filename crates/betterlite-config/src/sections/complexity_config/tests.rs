#![cfg(test)]

use std::collections::BTreeMap;

use crate::sections::Section;
use crate::sections::complexity_config::{Config, Raw, Settings};
use crate::sections::parse_section;

fn parse(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let mut ignored = Vec::new();
    let raw = parse_section::<Raw>(source, &mut ignored).map_err(|error| error.to_string())?;
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Section::convert(*raw, &lookup).map_err(|error| error.to_string())
}

// one deployment whose complexity router config holds the given keys, already indented under it
fn deployment(keys: &str) -> String {
    format!(
        "model_list:\n  - model_name: primary\n    litellm_params:\n      complexity_router_config:\n{keys}"
    )
}

fn only(keys: &str, variables: &[(&str, &str)]) -> Result<Option<Config>, String> {
    let mut deployments = parse(&deployment(keys), variables)?.deployments;
    let count = deployments.len();
    if count != 1 {
        return Err(format!("one deployment expected, got {count}"));
    }
    Ok(deployments.pop().flatten())
}

#[test]
fn a_deployment_without_the_keys_keeps_no_config() {
    let mut deployments = parse(
        "model_list:\n  - model_name: primary\n    litellm_params:\n      model: openai/gpt-4o\n",
        &[],
    )
    .unwrap()
    .deployments;
    assert_eq!(deployments.pop().unwrap(), None);
}

#[test]
fn every_key_resolves_from_its_written_shape() {
    let keys = "        escalation_keywords: [escalate, \"  hard ask  \"]
        plan_mode_min_tier: COMPLEX
        plan_mode_patterns: [Plan mode is active]
        housekeeping_patterns: [Write the title]
        heuristic_first_max_tier: MEDIUM
        hybrid_boundary_margin: 0.25
";
    let config = only(keys, &[]).unwrap().unwrap();
    assert_eq!(
        config.escalation_keywords,
        Some(vec!["escalate".to_owned(), "  hard ask  ".to_owned()])
    );
    assert_eq!(config.plan_mode_min_tier.as_deref(), Some("COMPLEX"));
    assert_eq!(
        config.plan_mode_patterns,
        Some(vec!["Plan mode is active".to_owned()])
    );
    assert_eq!(
        config.housekeeping_patterns,
        Some(vec!["Write the title".to_owned()])
    );
    assert_eq!(config.heuristic_first_max_tier.as_deref(), Some("MEDIUM"));
    assert_eq!(config.hybrid_boundary_margin, Some(0.25));
}

#[test]
fn absent_and_null_keys_stay_unset_while_an_empty_list_is_kept() {
    let keys = "        escalation_keywords: []
        plan_mode_min_tier: null
        plan_mode_patterns:
        housekeeping_patterns:
        heuristic_first_max_tier:
        hybrid_boundary_margin: null
";
    let config = only(keys, &[]).unwrap().unwrap();
    assert_eq!(config.escalation_keywords, Some(Vec::new()));
    assert_eq!(config.plan_mode_min_tier, None);
    assert_eq!(config.plan_mode_patterns, None);
    assert_eq!(config.housekeeping_patterns, None);
    assert_eq!(config.heuristic_first_max_tier, None);
    assert_eq!(config.hybrid_boundary_margin, None);
}

#[test]
fn an_integer_margin_reads_as_a_float() {
    let config = only("        hybrid_boundary_margin: 1\n", &[])
        .unwrap()
        .unwrap();
    assert_eq!(config.hybrid_boundary_margin, Some(1.0_f64));
}

#[test]
fn a_reference_in_a_scalar_key_resolves() {
    let keys = "        plan_mode_min_tier: os.environ/TIER\n        heuristic_first_max_tier: os.environ/THRESHOLD\n        hybrid_boundary_margin: os.environ/MARGIN\n";
    let config = only(
        keys,
        &[
            ("TIER", "COMPLEX"),
            ("THRESHOLD", "MEDIUM"),
            ("MARGIN", "0.5"),
        ],
    )
    .unwrap()
    .unwrap();
    assert_eq!(config.plan_mode_min_tier.as_deref(), Some("COMPLEX"));
    assert_eq!(config.heuristic_first_max_tier.as_deref(), Some("MEDIUM"));
    assert_eq!(config.hybrid_boundary_margin, Some(0.5));
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let keys = "        plan_mode_min_tier: os.environ/TIER\n        heuristic_first_max_tier: os.environ/THRESHOLD\n        hybrid_boundary_margin: os.environ/MARGIN\n";
    let config = only(keys, &[("TIER", ""), ("THRESHOLD", ""), ("MARGIN", "")])
        .unwrap()
        .unwrap();
    assert_eq!(config.plan_mode_min_tier, None);
    assert_eq!(config.heuristic_first_max_tier, None);
    assert_eq!(config.hybrid_boundary_margin, None);
}

#[test]
fn a_reference_in_a_list_entry_stays_literal() {
    let keys = "        escalation_keywords: [os.environ/KEYWORD]\n        plan_mode_patterns: [os.environ/PATTERN]\n";
    let config = only(keys, &[("KEYWORD", "escalate"), ("PATTERN", "Plan mode")])
        .unwrap()
        .unwrap();
    assert_eq!(
        config.escalation_keywords,
        Some(vec!["os.environ/KEYWORD".to_owned()])
    );
    assert_eq!(
        config.plan_mode_patterns,
        Some(vec!["os.environ/PATTERN".to_owned()])
    );
}

#[test]
fn a_quoted_margin_fails_the_load() {
    let error = only("        hybrid_boundary_margin: \"0.5\"\n", &[]).unwrap_err();
    assert!(error.contains("hybrid_boundary_margin"), "{error}");
}

#[test]
fn a_margin_naming_an_unset_variable_fails_the_load() {
    let error = only("        hybrid_boundary_margin: os.environ/MARGIN\n", &[]).unwrap_err();
    assert!(error.contains("MARGIN"), "{error}");
}

#[test]
fn deployments_stay_index_aligned() {
    let source = "model_list:
  - model_name: first
    litellm_params:
      model: openai/gpt-4o
  - model_name: second
    litellm_params:
      complexity_router_config:
        plan_mode_min_tier: COMPLEX
";
    let mut deployments = parse(source, &[]).unwrap().deployments;
    assert_eq!(deployments.len(), 2);
    assert_eq!(
        deployments.pop().unwrap().unwrap().plan_mode_min_tier,
        Some("COMPLEX".to_owned())
    );
    assert_eq!(deployments.pop().unwrap(), None);
}
