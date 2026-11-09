#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::tag_matchers::{DeploymentTags, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.tag_matchers)
        .map_err(|error| error.to_string())
}

fn config(params: &str) -> String {
    format!(
        "model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/gpt-4o\n{params}"
    )
}

#[test]
fn absent_tags_and_regex_stay_none() {
    let resolved = settings(&config(""), &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![DeploymentTags {
            tags: None,
            tag_regex: None,
        }]
    );
}

#[test]
fn explicit_lists_are_kept_in_order() {
    let resolved = settings(
        &config("      tags: [beta, alpha, default]\n      tag_regex: ['^User-Agent: a', 'b']"),
        &[],
    )
    .unwrap();
    assert_eq!(
        resolved.deployments,
        vec![DeploymentTags {
            tags: Some(vec![
                "beta".to_owned(),
                "alpha".to_owned(),
                "default".to_owned()
            ]),
            tag_regex: Some(vec!["^User-Agent: a".to_owned(), "b".to_owned()]),
        }]
    );
}

#[test]
fn an_explicit_empty_list_is_not_none() {
    let resolved = settings(&config("      tags: []\n      tag_regex: []"), &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![DeploymentTags {
            tags: Some(Vec::new()),
            tag_regex: Some(Vec::new()),
        }]
    );
}

#[test]
fn environment_references_stay_literal() {
    let resolved = settings(
        &config("      tags: [os.environ/TEAM_TAG, kept]\n      tag_regex: [os.environ/PATTERN]"),
        &[("TEAM_TAG", "team-a"), ("PATTERN", "^User-Agent:")],
    )
    .unwrap();
    assert_eq!(
        resolved.deployments,
        vec![DeploymentTags {
            tags: Some(vec!["os.environ/TEAM_TAG".to_owned(), "kept".to_owned()]),
            tag_regex: Some(vec!["os.environ/PATTERN".to_owned()]),
        }]
    );
}

#[test]
fn an_unset_environment_reference_stays_literal() {
    let resolved = settings(&config("      tags: [os.environ/ABSENT_TAG]"), &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].tags,
        Some(vec!["os.environ/ABSENT_TAG".to_owned()])
    );
}

#[test]
fn every_deployment_keeps_its_own_lists() {
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/a\n      tags: [one]\n  - model_name: b\n    litellm_params:\n      model: openai/b\n      tag_regex: [two]\n  - model_name: c\n    litellm_params:\n      model: openai/c\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(resolved.deployments.len(), 3);
    assert_eq!(resolved.deployments[0].tags, Some(vec!["one".to_owned()]));
    assert_eq!(resolved.deployments[0].tag_regex, None);
    assert_eq!(resolved.deployments[1].tags, None);
    assert_eq!(
        resolved.deployments[1].tag_regex,
        Some(vec!["two".to_owned()])
    );
    assert_eq!(resolved.deployments[2], DeploymentTags::default());
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/tag-matchers-valid.yaml");
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(resolved.deployments.len(), 2);
    assert_eq!(
        resolved.deployments[0].tags,
        Some(vec!["default".to_owned(), "team-a".to_owned()])
    );
    assert_eq!(
        resolved.deployments[0].tag_regex,
        Some(vec![
            "^User-Agent: claude-code/".to_owned(),
            "(?i)internal-tool".to_owned()
        ])
    );
    assert_eq!(resolved.deployments[1].tags, None);
}

#[test]
fn the_shipped_invalid_seed_is_rejected() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/tag-matchers-invalid.yaml");
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("tag_regex"), "{error}");
}
