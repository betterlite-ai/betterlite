#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::{
    applying_chain_members, get_matching_policies, policies_matching_conditions, policy_applies,
};
use crate::attachment::AttachmentRegistry;
use crate::context::PolicyMatchContext;
use crate::registry::PolicyRegistry;
use crate::values::{ModelPatterns, Policy, PolicyAttachment, PolicyCondition, PolicyGuardrails};

#[derive(Clone)]
struct InfoCapture(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for InfoCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::INFO {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn info_events<T, F>(run: F) -> (T, usize)
where
    F: FnOnce() -> T,
{
    let count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(InfoCapture(Arc::clone(&count)));
    let value = with_default(subscriber, run);
    (value, count.load(Ordering::Relaxed))
}

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

fn policies(entries: Vec<(&str, Policy)>) -> BTreeMap<String, Policy> {
    entries
        .into_iter()
        .map(|(name, policy)| (name.to_owned(), policy))
        .collect()
}

fn registry(policies: BTreeMap<String, Policy>) -> PolicyRegistry {
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
        scope: None,
        teams: None,
        keys: None,
        models: None,
        tags: None,
        priority: None,
        default: false,
    }
}

fn global(policy: &str) -> PolicyAttachment {
    let mut attachment = attachment(policy);
    attachment.scope = Some("*".to_owned());
    attachment
}

fn tagged(policy: &str, tag: &str) -> PolicyAttachment {
    let mut attachment = attachment(policy);
    attachment.tags = Some(names(&[tag]));
    attachment
}

fn attachments(entries: Vec<PolicyAttachment>) -> AttachmentRegistry {
    let mut registry = AttachmentRegistry::default();
    registry.load_attachments(entries);
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
fn get_matching_policies_keeps_attached_policies_whose_conditions_apply() {
    let policies = policies(vec![
        ("guardrail-y", policy(None, &["y"], None)),
        ("guardrail-x", policy(None, &["x"], Some("claude.*"))),
    ]);
    let attachments = attachments(vec![tagged("guardrail-x", "opt-in"), global("guardrail-y")]);
    let registry = registry(policies);
    let mut request = context("claude-haiku");
    request.tags = Some(names(&["opt-in"]));
    assert_eq!(
        get_matching_policies(&request, &attachments, &registry),
        ["guardrail-x"]
    );
}

#[test]
fn get_matching_policies_falls_back_to_a_default_attachment() {
    let mut fallback = global("guardrail-y");
    fallback.default = true;
    let policies = policies(vec![
        ("guardrail-y", policy(None, &["y"], None)),
        ("guardrail-x", policy(None, &["x"], Some("claude.*"))),
    ]);
    let attachments = attachments(vec![tagged("guardrail-x", "opt-in"), fallback]);
    let registry = registry(policies);
    let mut request = context("gpt-5.5");
    request.tags = Some(names(&["opt-in"]));
    assert_eq!(
        get_matching_policies(&request, &attachments, &registry),
        ["guardrail-y"]
    );
}

#[test]
fn get_matching_policies_returns_empty_without_loaded_attachments() {
    let policies = policies(vec![("baseline", policy(None, &["pii"], None))]);
    let registry = registry(policies);
    assert!(
        get_matching_policies(&context("gpt-4"), &AttachmentRegistry::default(), &registry)
            .is_empty()
    );
}

#[test]
fn get_matching_policies_returns_empty_without_loaded_policies() {
    let attachments = attachments(vec![global("baseline")]);
    assert!(
        get_matching_policies(&context("gpt-4"), &attachments, &PolicyRegistry::default())
            .is_empty()
    );
}

#[test]
fn policy_applies_needs_an_existing_policy_with_an_applying_member() {
    let policies = policies(vec![
        ("unconditional", policy(None, &["pii"], None)),
        ("claude-only", policy(None, &["x"], Some("claude.*"))),
    ]);
    let request = context("gpt-4");
    assert!(policy_applies("unconditional", &request, &policies));
    assert!(!policy_applies("claude-only", &request, &policies));
    assert!(!policy_applies("missing", &request, &policies));
}

#[test]
fn applying_chain_members_keeps_only_members_whose_condition_applies() {
    let policies = policies(vec![
        ("parent", policy(None, &["pii"], None)),
        ("child", policy(Some("parent"), &["x"], Some("claude.*"))),
    ]);
    let request = context("gpt-4");
    assert_eq!(
        applying_chain_members("child", &request, &policies),
        ["parent"]
    );
    assert_eq!(
        applying_chain_members("child", &context("claude-haiku"), &policies),
        ["parent", "child"]
    );
}

#[test]
fn an_ancestor_admits_a_child_whose_own_condition_fails() {
    let policies = policies(vec![
        ("parent", policy(None, &["pii"], None)),
        ("child", policy(Some("parent"), &["x"], Some("gpt-5.5"))),
    ]);
    let request = context("gpt-4o");
    let (applies, events) = info_events(|| policy_applies("child", &request, &policies));
    assert!(applies);
    assert_eq!(events, 1);
}

#[test]
fn no_info_log_when_the_policy_itself_applies() {
    let policies = policies(vec![
        ("parent", policy(None, &["pii"], None)),
        ("child", policy(Some("parent"), &["x"], Some("gpt-5.5"))),
    ]);
    let request = context("gpt-5.5");
    let (applies, events) = info_events(|| policy_applies("child", &request, &policies));
    assert!(applies);
    assert_eq!(events, 0);
}

#[test]
fn no_info_log_when_no_chain_member_applies() {
    let policies = policies(vec![
        ("parent", policy(None, &["pii"], Some("claude-opus-4-1"))),
        ("child", policy(Some("parent"), &["x"], Some("gpt-5.5"))),
    ]);
    let request = context("gpt-4o");
    let (applies, events) = info_events(|| policy_applies("child", &request, &policies));
    assert!(!applies);
    assert_eq!(events, 0);
}

#[test]
fn policies_matching_conditions_filters_in_input_order() {
    let policies = policies(vec![
        ("a", policy(None, &["x"], Some("gpt.*"))),
        ("b", policy(None, &["y"], None)),
    ]);
    let requested = names(&["b", "missing", "a"]);
    let matching = policies_matching_conditions(&requested, &context("claude-3"), &policies);
    assert_eq!(matching, ["b"]);
}

#[test]
fn policies_matching_conditions_keeps_a_child_with_an_applying_parent() {
    let policies = policies(vec![
        ("parent", policy(None, &["pii"], None)),
        ("child", policy(Some("parent"), &["x"], Some("claude.*"))),
    ]);
    let requested = names(&["child"]);
    assert_eq!(
        policies_matching_conditions(&requested, &context("gpt-4"), &policies),
        ["child"]
    );
}
