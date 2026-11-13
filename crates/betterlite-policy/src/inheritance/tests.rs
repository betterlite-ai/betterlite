#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::{resolve_inheritance_chain, resolve_policy_guardrails};
use crate::context::PolicyMatchContext;
use crate::values::{ModelPatterns, Policy, PolicyCondition, PolicyGuardrails};

#[derive(Clone)]
struct WarnCapture(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for WarnCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn warn_events<T, F>(run: F) -> (T, usize)
where
    F: FnOnce() -> T,
{
    let count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(WarnCapture(Arc::clone(&count)));
    let value = with_default(subscriber, run);
    (value, count.load(Ordering::Relaxed))
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn policy(inherit: Option<&str>, add: &[&str], remove: &[&str], model: Option<&str>) -> Policy {
    Policy {
        inherit: inherit.map(str::to_owned),
        description: None,
        guardrails: PolicyGuardrails {
            add: Some(names(add)),
            remove: Some(names(remove)),
        },
        condition: model.map(|pattern| PolicyCondition {
            model: Some(ModelPatterns::One(pattern.to_owned())),
        }),
        pipeline: None,
    }
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
fn the_chain_runs_from_the_root_to_the_policy() {
    let policies = BTreeMap::from([
        ("base".to_owned(), policy(None, &["pii"], &[], None)),
        (
            "child".to_owned(),
            policy(Some("base"), &["hipaa"], &[], None),
        ),
    ]);
    assert_eq!(
        resolve_inheritance_chain("child", &policies),
        ["base", "child"]
    );
}

#[test]
fn an_unknown_policy_has_an_empty_chain() {
    let policies = BTreeMap::from([("base".to_owned(), policy(None, &[], &[], None))]);
    assert!(resolve_inheritance_chain("missing", &policies).is_empty());
    assert!(resolve_inheritance_chain("missing", &BTreeMap::new()).is_empty());
}

#[test]
fn a_cycle_returns_the_partial_chain_with_a_warning() {
    let policies = BTreeMap::from([
        ("a".to_owned(), policy(Some("b"), &[], &[], None)),
        ("b".to_owned(), policy(Some("a"), &[], &[], None)),
    ]);
    let (chain, warnings) = warn_events(|| resolve_inheritance_chain("a", &policies));
    assert_eq!(chain, ["b", "a"]);
    assert_eq!(warnings, 1);
}

#[test]
fn self_inheritance_returns_the_policy_with_a_warning() {
    let policies = BTreeMap::from([("a".to_owned(), policy(Some("a"), &[], &[], None))]);
    let (chain, warnings) = warn_events(|| resolve_inheritance_chain("a", &policies));
    assert_eq!(chain, ["a"]);
    assert_eq!(warnings, 1);
}

#[test]
fn a_missing_parent_contributes_nothing() {
    let policies = BTreeMap::from([("child".to_owned(), policy(Some("missing"), &[], &[], None))]);
    let (chain, warnings) = warn_events(|| resolve_inheritance_chain("child", &policies));
    assert_eq!(chain, ["child"]);
    assert_eq!(warnings, 0);
}

#[test]
fn an_empty_inherit_string_is_no_parent() {
    let policies = BTreeMap::from([("child".to_owned(), policy(Some(""), &[], &[], None))]);
    assert_eq!(resolve_inheritance_chain("child", &policies), ["child"]);
}

#[test]
fn applies_add_then_remove_root_first() {
    let policies = BTreeMap::from([
        ("base".to_owned(), policy(None, &["pii"], &[], None)),
        (
            "child".to_owned(),
            policy(Some("base"), &["hipaa"], &["pii"], None),
        ),
    ]);
    let resolved = resolve_policy_guardrails("child", &policies, None);
    assert_eq!(resolved.policy_name, "child");
    assert_eq!(resolved.guardrails, ["hipaa"]);
    assert_eq!(resolved.inheritance_chain, ["base", "child"]);
}

#[test]
fn a_policy_removes_what_it_adds_in_the_same_step() {
    let policies = BTreeMap::from([(
        "base".to_owned(),
        policy(None, &["audit"], &["audit"], None),
    )]);
    assert!(
        resolve_policy_guardrails("base", &policies, None)
            .guardrails
            .is_empty()
    );
}

#[test]
fn a_child_removes_an_inherited_guardrail() {
    let policies = BTreeMap::from([
        ("base".to_owned(), policy(None, &["pii"], &[], None)),
        (
            "child".to_owned(),
            policy(Some("base"), &[], &["pii"], None),
        ),
    ]);
    assert!(
        resolve_policy_guardrails("child", &policies, None)
            .guardrails
            .is_empty()
    );
}

#[test]
fn returns_the_guardrails_sorted() {
    let policies = BTreeMap::from([
        (
            "base".to_owned(),
            policy(None, &["zeta", "alpha"], &[], None),
        ),
        (
            "child".to_owned(),
            policy(Some("base"), &["beta"], &[], None),
        ),
    ]);
    assert_eq!(
        resolve_policy_guardrails("child", &policies, None).guardrails,
        ["alpha", "beta", "zeta"]
    );
}

#[test]
fn skips_a_chain_policy_whose_condition_fails() {
    let policies = BTreeMap::from([
        ("base".to_owned(), policy(None, &["pii"], &[], None)),
        (
            "child".to_owned(),
            policy(Some("base"), &["hipaa"], &["pii"], Some("gpt-4")),
        ),
    ]);
    let resolved = resolve_policy_guardrails("child", &policies, Some(&context("claude-3")));
    assert_eq!(resolved.guardrails, ["pii"]);
    assert_eq!(resolved.inheritance_chain, ["base", "child"]);
}

#[test]
fn includes_a_chain_policy_whose_condition_matches() {
    let policies = BTreeMap::from([
        ("base".to_owned(), policy(None, &["pii"], &[], None)),
        (
            "child".to_owned(),
            policy(Some("base"), &["hipaa"], &["pii"], Some("gpt-4")),
        ),
    ]);
    let resolved = resolve_policy_guardrails("child", &policies, Some(&context("gpt-4")));
    assert_eq!(resolved.guardrails, ["hipaa"]);
}

#[test]
fn ignores_conditions_when_no_context_is_passed() {
    let policies = BTreeMap::from([
        ("base".to_owned(), policy(None, &["pii"], &[], None)),
        (
            "child".to_owned(),
            policy(Some("base"), &["hipaa"], &["pii"], Some("gpt-4")),
        ),
    ]);
    let resolved = resolve_policy_guardrails("child", &policies, None);
    assert_eq!(resolved.guardrails, ["hipaa"]);
}
