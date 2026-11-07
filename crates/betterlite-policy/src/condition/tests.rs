#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::evaluate_condition;
use crate::context::PolicyMatchContext;
use crate::values::{ModelPatterns, PolicyCondition};

#[derive(Clone)]
struct DebugCapture(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn debug_events<T, F>(run: F) -> (T, usize)
where
    F: FnOnce() -> T,
{
    let count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(DebugCapture(Arc::clone(&count)));
    let value = with_default(subscriber, run);
    (value, count.load(Ordering::Relaxed))
}

fn context(model: Option<&str>) -> PolicyMatchContext {
    PolicyMatchContext {
        team_alias: None,
        key_alias: None,
        model: model.map(str::to_owned),
        tags: None,
    }
}

fn one(pattern: &str) -> PolicyCondition {
    PolicyCondition {
        model: Some(ModelPatterns::One(pattern.to_owned())),
    }
}

fn many(patterns: &[&str]) -> PolicyCondition {
    PolicyCondition {
        model: Some(ModelPatterns::Many(
            patterns
                .iter()
                .map(|pattern| String::from(*pattern))
                .collect(),
        )),
    }
}

#[test]
fn no_condition_matches_every_model() {
    assert!(evaluate_condition(None, &context(Some("gpt-4"))));
    assert!(evaluate_condition(None, &context(None)));
}

#[test]
fn a_condition_without_a_model_matches() {
    let condition = PolicyCondition { model: None };
    assert!(evaluate_condition(
        Some(&condition),
        &context(Some("gpt-4"))
    ));
    assert!(evaluate_condition(Some(&condition), &context(None)));
}

#[test]
fn a_missing_context_model_does_not_match() {
    assert!(!evaluate_condition(Some(&one("gpt-4")), &context(None)));
}

#[test]
fn matches_an_exact_string() {
    assert!(evaluate_condition(
        Some(&one("gpt-4")),
        &context(Some("gpt-4"))
    ));
    assert!(!evaluate_condition(
        Some(&one("gpt-4")),
        &context(Some("gpt-4o"))
    ));
}

#[test]
fn matches_a_whole_value_regex() {
    assert!(evaluate_condition(
        Some(&one("gpt-4.*")),
        &context(Some("gpt-4-turbo"))
    ));
}

#[test]
fn matches_any_pattern_in_a_list() {
    assert!(evaluate_condition(
        Some(&many(&["claude-3", "gpt-4.*"])),
        &context(Some("gpt-4o"))
    ));
    assert!(!evaluate_condition(
        Some(&many(&["claude-3", "gemini-.*"])),
        &context(Some("gpt-4o"))
    ));
}

#[test]
fn a_partial_regex_does_not_match() {
    assert!(!evaluate_condition(
        Some(&one("gpt-4")),
        &context(Some("gpt-4-turbo"))
    ));
}

#[test]
fn an_invalid_regex_falls_back_to_exact_equality() {
    assert!(evaluate_condition(
        Some(&one("gpt-4(")),
        &context(Some("gpt-4("))
    ));
    assert!(!evaluate_condition(
        Some(&one("gpt-4(")),
        &context(Some("gpt-4"))
    ));
}

#[test]
fn an_empty_pattern_list_never_matches() {
    assert!(!evaluate_condition(
        Some(&many(&[])),
        &context(Some("gpt-4"))
    ));
    assert!(!evaluate_condition(Some(&many(&[])), &context(None)));
}

#[test]
fn logs_a_debug_line_when_the_model_check_fails() {
    let (matched, events) =
        debug_events(|| evaluate_condition(Some(&one("gpt-4")), &context(Some("gpt-4-turbo"))));
    assert!(!matched);
    assert_eq!(events, 1);

    let (hit, count) =
        debug_events(|| evaluate_condition(Some(&one("gpt-4")), &context(Some("gpt-4"))));
    assert!(hit);
    assert_eq!(count, 0);
}
