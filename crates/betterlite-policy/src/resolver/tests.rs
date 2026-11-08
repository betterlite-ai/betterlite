#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    pipeline_managed_guardrails, resolve_guardrails_for_context, resolve_pipelines_for_context,
};
use crate::attachment::AttachmentRegistry;
use crate::context::PolicyMatchContext;
use crate::pipeline::{GuardrailPipeline, PipelineAction, PipelineMode, PipelineStep};
use crate::registry::PolicyRegistry;
use crate::values::{ModelPatterns, Policy, PolicyAttachment, PolicyCondition, PolicyGuardrails};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn policy(inherit: Option<&str>, add: &[&str], model: Option<&str>) -> Policy {
    Policy {
        inherit: inherit.map(str::to_owned),
        description: None,
        guardrails: PolicyGuardrails {
            add: Some(names(add)),
            remove: None,
        },
        condition: model.map(|pattern| PolicyCondition {
            model: Some(ModelPatterns::One(pattern.to_owned())),
        }),
        pipeline: None,
    }
}

fn pipeline(mode: PipelineMode, guardrails: &[&str]) -> GuardrailPipeline {
    let steps = guardrails
        .iter()
        .map(|guardrail| PipelineStep {
            guardrail: String::from(*guardrail),
            on_fail: PipelineAction::Block,
            on_pass: PipelineAction::Allow,
            on_error: None,
            pass_data: false,
            modify_response_message: None,
        })
        .collect();
    GuardrailPipeline { mode, steps }
}

fn with_pipeline(mut policy: Policy, mode: PipelineMode, guardrails: &[&str]) -> Policy {
    policy.pipeline = Some(pipeline(mode, guardrails));
    policy
}

fn registry(entries: Vec<(&str, Policy)>) -> PolicyRegistry {
    let policies: BTreeMap<String, Policy> = entries
        .into_iter()
        .map(|(name, policy)| (name.to_owned(), policy))
        .collect();
    PolicyRegistry {
        config_policies: policies.clone(),
        policies,
        sources: BTreeMap::new(),
        policies_by_id: BTreeMap::new(),
        initialized: true,
    }
}

fn attachment(policy: &str) -> PolicyAttachment {
    PolicyAttachment {
        policy: policy.to_owned(),
        scope: Some("*".to_owned()),
        teams: None,
        keys: None,
        models: None,
        tags: None,
        priority: None,
        default: false,
    }
}

fn attachments(policies: &[&str]) -> AttachmentRegistry {
    let mut registry = AttachmentRegistry::default();
    registry.load_attachments(policies.iter().map(|policy| attachment(policy)).collect());
    registry
}

fn context(model: &str) -> PolicyMatchContext {
    PolicyMatchContext {
        team_alias: None,
        key_alias: None,
        model: Some(model.to_owned()),
        tags: None,
    }
}

#[test]
fn the_union_of_matching_policies_is_sorted() {
    let registry = registry(vec![
        ("first", policy(None, &["pii", "audit"], None)),
        ("second", policy(None, &["toxicity", "pii"], None)),
    ]);
    let attachments = attachments(&["first", "second"]);
    assert_eq!(
        resolve_guardrails_for_context(&context("gpt-4"), &registry, &attachments, None),
        ["audit", "pii", "toxicity"]
    );
}

#[test]
fn explicit_policy_names_skip_attachment_matching() {
    let registry = registry(vec![("second", policy(None, &["toxicity"], None))]);
    let attachments = AttachmentRegistry::default();
    let requested = names(&["second"]);
    assert_eq!(
        resolve_guardrails_for_context(
            &context("gpt-4"),
            &registry,
            &attachments,
            Some(&requested)
        ),
        ["toxicity"]
    );
}

#[test]
fn a_failing_child_contributes_only_its_applying_ancestor() {
    let registry = registry(vec![
        ("parent", policy(None, &["pii"], None)),
        (
            "child",
            policy(Some("parent"), &["hipaa"], Some("claude.*")),
        ),
    ]);
    let attachments = attachments(&["child"]);
    assert_eq!(
        resolve_guardrails_for_context(&context("gpt-4"), &registry, &attachments, None),
        ["pii"]
    );
}

#[test]
fn no_matching_policy_gives_no_guardrails() {
    let registry = registry(vec![("baseline", policy(None, &["pii"], None))]);
    let mut scoped_attachments = AttachmentRegistry::default();
    let mut scoped = attachment("baseline");
    scoped.scope = None;
    scoped.teams = Some(names(&["team-a"]));
    scoped_attachments.load_attachments(vec![scoped]);
    assert!(
        resolve_guardrails_for_context(&context("gpt-4"), &registry, &scoped_attachments, None)
            .is_empty()
    );
}

#[test]
fn an_uninitialized_policy_registry_resolves_nothing() {
    let attachments = attachments(&["baseline"]);
    assert!(
        resolve_guardrails_for_context(
            &context("gpt-4"),
            &PolicyRegistry::default(),
            &attachments,
            None
        )
        .is_empty()
    );
}

#[test]
fn resolve_pipelines_returns_pipelines_for_matching_policies() {
    let registry = registry(vec![
        (
            "pipeline-policy",
            with_pipeline(
                policy(None, &["pii"], None),
                PipelineMode::PreCall,
                &["pii", "audit"],
            ),
        ),
        ("flat-policy", policy(None, &["toxicity"], None)),
    ]);
    let attachments = attachments(&["pipeline-policy", "flat-policy"]);
    let pipelines = resolve_pipelines_for_context(&context("gpt-4"), &registry, &attachments, None);
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].0, "pipeline-policy");
    assert_eq!(pipelines[0].1.mode, PipelineMode::PreCall);
    assert_eq!(
        pipelines[0]
            .1
            .steps
            .iter()
            .map(|step| step.guardrail.as_str())
            .collect::<Vec<_>>(),
        ["pii", "audit"]
    );
}

#[test]
fn resolve_pipelines_skips_a_policy_whose_condition_fails() {
    let registry = registry(vec![
        (
            "claude-pipeline",
            with_pipeline(
                policy(None, &["pii"], Some("claude.*")),
                PipelineMode::PostCall,
                &["pii"],
            ),
        ),
        (
            "flat-pipeline",
            with_pipeline(
                policy(None, &["audit"], None),
                PipelineMode::PostCall,
                &["audit"],
            ),
        ),
    ]);
    let attachments = attachments(&["claude-pipeline", "flat-pipeline"]);
    let pipelines = resolve_pipelines_for_context(&context("gpt-4"), &registry, &attachments, None);
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].0, "flat-pipeline");
}

#[test]
fn resolve_pipelines_ignores_unknown_policy_names() {
    let registry = registry(vec![(
        "pipeline-policy",
        with_pipeline(
            policy(None, &["pii"], None),
            PipelineMode::PreCall,
            &["pii"],
        ),
    )]);
    let attachments = AttachmentRegistry::default();
    let requested = names(&["ghost", "pipeline-policy"]);
    let pipelines =
        resolve_pipelines_for_context(&context("gpt-4"), &registry, &attachments, Some(&requested));
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].0, "pipeline-policy");
}

#[test]
fn pipeline_managed_guardrails_collects_every_step() {
    let pipelines = vec![
        (
            "first".to_owned(),
            pipeline(PipelineMode::PreCall, &["pii", "audit"]),
        ),
        (
            "second".to_owned(),
            pipeline(PipelineMode::PostCall, &["audit"]),
        ),
    ];
    assert_eq!(
        pipeline_managed_guardrails(&pipelines)
            .into_iter()
            .collect::<Vec<_>>(),
        ["audit", "pii"]
    );
}
