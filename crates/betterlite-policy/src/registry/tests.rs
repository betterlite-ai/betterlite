#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    POLICY_VERSION_ID_PREFIX, PolicyRegistry, PolicySource, RawGuardrailLists, RawGuardrails,
    RawPipeline, RawPolicy, parse_pipeline, parse_policy,
};
use crate::pipeline::{
    GuardrailPipeline, PipelineAction, PipelineError, PipelineMode, PipelineStep,
};
use crate::values::{ModelPatterns, Policy, PolicyCondition, PolicyGuardrails};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn condition(model: &str) -> PolicyCondition {
    PolicyCondition {
        model: Some(ModelPatterns::One(model.to_owned())),
    }
}

fn step(guardrail: &str) -> PipelineStep {
    PipelineStep {
        guardrail: guardrail.to_owned(),
        on_fail: PipelineAction::Block,
        on_pass: PipelineAction::Allow,
        on_error: None,
        pass_data: false,
        modify_response_message: None,
    }
}

fn raw(inherit: Option<&str>, add: &[&str], remove: &[&str]) -> RawPolicy {
    RawPolicy {
        inherit: inherit.map(str::to_owned),
        description: None,
        guardrails: Some(RawGuardrails::Lists(RawGuardrailLists {
            add: Some(names(add)),
            remove: Some(names(remove)),
        })),
        condition: None,
        pipeline: None,
    }
}

fn config(entries: Vec<(&str, RawPolicy)>) -> BTreeMap<String, RawPolicy> {
    entries
        .into_iter()
        .map(|(name, policy)| (name.to_owned(), policy))
        .collect()
}

fn runtime_policy(inherit: Option<&str>, add: &[&str]) -> Policy {
    parse_policy("added", raw(inherit, add, &[])).unwrap()
}

#[test]
fn the_version_id_prefix_is_policy_underscore() {
    assert_eq!(POLICY_VERSION_ID_PREFIX, "policy_");
}

#[test]
fn parse_policy_reads_every_field() {
    let data = RawPolicy {
        inherit: Some("baseline".to_owned()),
        description: Some("Healthcare".to_owned()),
        guardrails: Some(RawGuardrails::Lists(RawGuardrailLists {
            add: Some(names(&["hipaa", "audit"])),
            remove: Some(names(&["pii"])),
        })),
        condition: Some(condition("gpt-4")),
        pipeline: Some(RawPipeline {
            mode: PipelineMode::PostCall,
            steps: vec![step("hipaa")],
        }),
    };
    let parsed = parse_policy("healthcare", data).unwrap();
    assert_eq!(parsed.inherit.as_deref(), Some("baseline"));
    assert_eq!(parsed.description.as_deref(), Some("Healthcare"));
    assert_eq!(parsed.guardrails.add, Some(names(&["hipaa", "audit"])));
    assert_eq!(parsed.guardrails.remove, Some(names(&["pii"])));
    assert_eq!(parsed.condition, Some(condition("gpt-4")));
    assert_eq!(
        parsed.pipeline,
        Some(GuardrailPipeline {
            mode: PipelineMode::PostCall,
            steps: vec![step("hipaa")],
        })
    );
}

#[test]
fn parse_policy_turns_a_guardrails_list_into_add() {
    let data = RawPolicy {
        inherit: None,
        description: None,
        guardrails: Some(RawGuardrails::Add(names(&["pii", "phi"]))),
        condition: None,
        pipeline: None,
    };
    let parsed = parse_policy("legacy", data).unwrap();
    assert_eq!(parsed.guardrails.add, Some(names(&["pii", "phi"])));
    assert_eq!(parsed.guardrails.remove, None);
}

#[test]
fn parse_policy_turns_an_empty_guardrails_list_into_no_add() {
    let data = RawPolicy {
        inherit: None,
        description: None,
        guardrails: Some(RawGuardrails::Add(Vec::new())),
        condition: None,
        pipeline: None,
    };
    let parsed = parse_policy("legacy", data).unwrap();
    assert_eq!(parsed.guardrails, PolicyGuardrails::default());
}

#[test]
fn parse_policy_leaves_guardrails_empty_when_absent() {
    let data = RawPolicy {
        inherit: None,
        description: None,
        guardrails: None,
        condition: None,
        pipeline: None,
    };
    assert_eq!(
        parse_policy("plain", data).unwrap().guardrails,
        PolicyGuardrails::default()
    );
}

#[test]
fn parse_policy_leaves_guardrails_empty_for_an_empty_object() {
    let data = RawPolicy {
        inherit: None,
        description: None,
        guardrails: Some(RawGuardrails::Lists(RawGuardrailLists {
            add: None,
            remove: None,
        })),
        condition: None,
        pipeline: None,
    };
    assert_eq!(
        parse_policy("plain", data).unwrap().guardrails,
        PolicyGuardrails::default()
    );
}

#[test]
fn parse_pipeline_defaults_the_mode_to_pre_call_and_fills_a_bare_step() {
    let data: RawPipeline = serde_json::from_str(r#"{"steps":[{"guardrail":"pii"}]}"#).unwrap();
    assert_eq!(data.mode, PipelineMode::PreCall);
    let parsed = parse_pipeline(data).unwrap();
    assert_eq!(
        parsed,
        GuardrailPipeline {
            mode: PipelineMode::PreCall,
            steps: vec![PipelineStep {
                guardrail: "pii".to_owned(),
                on_fail: PipelineAction::Block,
                on_pass: PipelineAction::Allow,
                on_error: None,
                pass_data: false,
                modify_response_message: None,
            }],
        }
    );
}

#[test]
fn parse_pipeline_keeps_an_explicit_mode() {
    let parsed = parse_pipeline(RawPipeline {
        mode: PipelineMode::PostCall,
        steps: vec![step("hipaa")],
    })
    .unwrap();
    assert_eq!(parsed.mode, PipelineMode::PostCall);
}

#[test]
fn parse_pipeline_rejects_an_empty_step_list() {
    let error = parse_pipeline(RawPipeline {
        mode: PipelineMode::PreCall,
        steps: Vec::new(),
    })
    .unwrap_err();
    assert!(matches!(error, PipelineError::NoSteps));
}

#[test]
fn parse_policy_names_the_policy_in_a_pipeline_error() {
    let data = RawPolicy {
        inherit: None,
        description: None,
        guardrails: None,
        condition: None,
        pipeline: Some(RawPipeline {
            mode: PipelineMode::PreCall,
            steps: Vec::new(),
        }),
    };
    let error = parse_policy("broken", data).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid policy 'broken': List should have at least 1 item after validation, not 0"
    );
}

#[test]
fn load_policies_tracks_config_policies_and_sources() {
    let mut registry = PolicyRegistry::default();
    registry
        .load_policies(config(vec![
            ("baseline", raw(None, &["pii"], &[])),
            ("healthcare", raw(Some("baseline"), &["hipaa"], &["pii"])),
        ]))
        .unwrap();
    assert!(registry.initialized);
    assert_eq!(
        registry.policies.keys().cloned().collect::<Vec<_>>(),
        ["baseline", "healthcare"]
    );
    assert_eq!(
        registry.sources.get("baseline"),
        Some(&PolicySource::Config)
    );
    assert_eq!(
        registry.sources.get("healthcare"),
        Some(&PolicySource::Config)
    );
    assert_eq!(registry.config_policies, registry.policies);
}

#[test]
fn load_policies_clears_the_previous_state() {
    let mut registry = PolicyRegistry::default();
    registry.add_policy(
        "db-only",
        runtime_policy(None, &["toxicity"]),
        PolicySource::Db,
    );
    registry.policies_by_id.insert(
        "version-1".to_owned(),
        ("db-only".to_owned(), runtime_policy(None, &[])),
    );
    registry
        .load_policies(config(vec![("baseline", raw(None, &["pii"], &[]))]))
        .unwrap();
    assert_eq!(
        registry.policies.keys().cloned().collect::<Vec<_>>(),
        ["baseline"]
    );
    assert!(registry.policies_by_id.is_empty());
    assert!(!registry.config_policies.contains_key("db-only"));
}

#[test]
fn add_policy_records_a_db_source() {
    let mut registry = PolicyRegistry::default();
    registry.add_policy(
        "custom",
        runtime_policy(None, &["toxicity"]),
        PolicySource::Db,
    );
    assert!(registry.initialized);
    assert_eq!(registry.sources.get("custom"), Some(&PolicySource::Db));
    assert!(!registry.config_policies.contains_key("custom"));
}

#[test]
fn add_policy_with_a_config_source_updates_the_config_snapshot() {
    let mut registry = PolicyRegistry::default();
    let added = runtime_policy(None, &["toxicity"]);
    registry.add_policy("custom", added.clone(), PolicySource::Config);
    assert_eq!(registry.config_policies.get("custom"), Some(&added));
}

#[test]
fn add_policy_replaces_an_earlier_version() {
    let mut registry = PolicyRegistry::default();
    registry.add_policy("custom", runtime_policy(None, &["first"]), PolicySource::Db);
    registry.add_policy(
        "custom",
        runtime_policy(None, &["second"]),
        PolicySource::Db,
    );
    assert_eq!(
        registry
            .policies
            .get("custom")
            .and_then(|policy| policy.guardrails.add.clone()),
        Some(names(&["second"]))
    );
}

#[test]
fn remove_policy_restores_the_config_version() {
    let mut registry = PolicyRegistry::default();
    registry
        .load_policies(config(vec![("baseline", raw(None, &["pii"], &[]))]))
        .unwrap();
    registry.add_policy(
        "baseline",
        runtime_policy(None, &["from-db"]),
        PolicySource::Db,
    );
    assert!(registry.remove_policy("baseline"));
    assert_eq!(
        registry
            .policies
            .get("baseline")
            .and_then(|policy| policy.guardrails.add.clone()),
        Some(names(&["pii"]))
    );
    assert_eq!(
        registry.sources.get("baseline"),
        Some(&PolicySource::Config)
    );
}

#[test]
fn remove_policy_restores_a_config_policy_that_was_never_overridden() {
    let mut registry = PolicyRegistry::default();
    registry
        .load_policies(config(vec![("baseline", raw(None, &["pii"], &[]))]))
        .unwrap();
    assert!(registry.remove_policy("baseline"));
    assert!(registry.policies.contains_key("baseline"));
    assert_eq!(
        registry.sources.get("baseline"),
        Some(&PolicySource::Config)
    );
}

#[test]
fn remove_policy_deletes_a_db_only_policy() {
    let mut registry = PolicyRegistry::default();
    registry.add_policy(
        "custom",
        runtime_policy(None, &["toxicity"]),
        PolicySource::Db,
    );
    assert!(registry.remove_policy("custom"));
    assert!(!registry.policies.contains_key("custom"));
    assert!(!registry.sources.contains_key("custom"));
}

#[test]
fn remove_policy_returns_false_for_an_unknown_name() {
    let mut registry = PolicyRegistry::default();
    registry
        .load_policies(config(vec![("baseline", raw(None, &["pii"], &[]))]))
        .unwrap();
    assert!(!registry.remove_policy("ghost"));
    assert!(registry.policies.contains_key("baseline"));
}

#[test]
fn policies_by_id_holds_request_versions() {
    let mut registry = PolicyRegistry::default();
    let versioned = runtime_policy(None, &["pii"]);
    registry.policies_by_id.insert(
        "2f7c".to_owned(),
        ("baseline".to_owned(), versioned.clone()),
    );
    assert_eq!(
        registry.policies_by_id.get("2f7c"),
        Some(&("baseline".to_owned(), versioned))
    );
    assert_eq!(registry.policies_by_id.get("missing"), None);
}
