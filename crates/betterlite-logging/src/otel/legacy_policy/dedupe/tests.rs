#![cfg(test)]

use std::collections::BTreeMap;
use std::slice;

use super::{
    EmittedSpans, FloatKey, FrozenScope, HandlerIdentity, MAX_FREEZE_DEPTH, ScopeValue,
    freeze_scope,
};

fn handler(class_name: &str, instance: u64) -> HandlerIdentity {
    HandlerIdentity {
        class_name: class_name.to_owned(),
        instance,
    }
}

fn text(value: &str) -> ScopeValue {
    ScopeValue::Text(value.to_owned())
}

fn nested(depth: u32) -> ScopeValue {
    if depth == 0 {
        return text("leaf");
    }
    ScopeValue::List(vec![nested(depth.saturating_sub(1))])
}

fn deepest_json(value: &FrozenScope) -> Option<&str> {
    match value {
        FrozenScope::Json(encoded) => Some(encoded),
        FrozenScope::List(items) => items.iter().find_map(deepest_json),
        FrozenScope::Null
        | FrozenScope::Bool(_)
        | FrozenScope::Int(_)
        | FrozenScope::Float(_)
        | FrozenScope::Text(_)
        | FrozenScope::Set(_)
        | FrozenScope::Map(_) => None,
    }
}

#[test]
fn emit_once_accepts_the_first_call_and_drops_repeats() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    let scope = vec![text("success")];
    assert!(spans.emit_once(&handler, &scope));
    assert!(!spans.emit_once(&handler, &scope));
}

#[test]
fn different_handlers_emit_their_own_spans() {
    let mut spans = EmittedSpans::default();
    let scope = vec![text("success")];
    assert!(spans.emit_once(&handler("OpenTelemetry", 1), &scope));
    assert!(spans.emit_once(&handler("Langfuse", 1), &scope));
    assert!(spans.emit_once(&handler("OpenTelemetry", 2), &scope));
    assert!(!spans.emit_once(&handler("OpenTelemetry", 1), &scope));
}

#[test]
fn different_scopes_are_separate_entries() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    assert!(spans.emit_once(&handler, &[text("guardrail"), text("pii")]));
    assert!(spans.emit_once(&handler, &[text("guardrail"), text("secrets")]));
    assert!(!spans.emit_once(&handler, &[text("guardrail"), text("pii")]));
}

#[test]
fn lists_keep_their_order() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    let forward = ScopeValue::List(vec![ScopeValue::Int(1), ScopeValue::Int(2)]);
    let backward = ScopeValue::List(vec![ScopeValue::Int(2), ScopeValue::Int(1)]);
    assert!(spans.emit_once(&handler, slice::from_ref(&forward)));
    assert!(spans.emit_once(&handler, slice::from_ref(&backward)));
}

#[test]
fn sets_ignore_their_order() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    let forward = ScopeValue::Set(vec![ScopeValue::Int(1), ScopeValue::Int(2)]);
    let backward = ScopeValue::Set(vec![ScopeValue::Int(2), ScopeValue::Int(1)]);
    assert!(spans.emit_once(&handler, slice::from_ref(&forward)));
    assert!(!spans.emit_once(&handler, slice::from_ref(&backward)));
}

#[test]
fn maps_ignore_their_order_and_keep_their_values() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    let first = ScopeValue::Map(BTreeMap::from([
        ("name".to_owned(), text("pii")),
        ("mode".to_owned(), text("pre")),
    ]));
    let second = ScopeValue::Map(BTreeMap::from([
        ("mode".to_owned(), text("pre")),
        ("name".to_owned(), text("pii")),
    ]));
    let other = ScopeValue::Map(BTreeMap::from([("name".to_owned(), text("pii"))]));
    assert!(spans.emit_once(&handler, slice::from_ref(&first)));
    assert!(!spans.emit_once(&handler, slice::from_ref(&second)));
    assert!(spans.emit_once(&handler, slice::from_ref(&other)));
}

#[test]
fn negative_zero_and_zero_share_one_key() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    assert!(spans.emit_once(&handler, &[ScopeValue::Float(-0.0)]));
    assert!(!spans.emit_once(&handler, &[ScopeValue::Float(0.0)]));
}

#[test]
fn values_past_the_depth_limit_collapse_to_json_text() {
    let deep = nested(MAX_FREEZE_DEPTH.saturating_add(1));
    let frozen = freeze_scope(&deep, 0);
    assert_eq!(deepest_json(&frozen), Some(r#"["leaf"]"#));
}

#[test]
fn a_collapsed_value_still_keys_on_its_content() {
    let mut spans = EmittedSpans::default();
    let handler = handler("OpenTelemetry", 1);
    let deep = nested(MAX_FREEZE_DEPTH.saturating_add(2));
    let mut other = nested(MAX_FREEZE_DEPTH.saturating_add(2));
    assert!(spans.emit_once(&handler, slice::from_ref(&deep)));
    assert!(!spans.emit_once(&handler, slice::from_ref(&other)));
    other = ScopeValue::List(vec![other, text("extra")]);
    assert!(spans.emit_once(&handler, slice::from_ref(&other)));
}

#[test]
fn scalars_freeze_to_themselves() {
    assert_eq!(freeze_scope(&ScopeValue::Null, 0), FrozenScope::Null);
    assert_eq!(
        freeze_scope(&ScopeValue::Bool(true), 0),
        FrozenScope::Bool(true)
    );
    assert_eq!(freeze_scope(&ScopeValue::Int(7), 0), FrozenScope::Int(7));
    assert_eq!(
        freeze_scope(&ScopeValue::Float(1.5), 0),
        FrozenScope::Float(FloatKey::from(1.5_f64))
    );
    assert_eq!(
        freeze_scope(&text("value"), 0),
        FrozenScope::Text("value".to_owned())
    );
}
