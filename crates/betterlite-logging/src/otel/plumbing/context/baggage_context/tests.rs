#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};

use crate::otel::plumbing::context::baggage_context::set_request_baggage;

fn values(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn baggage_entry(context: &Context, key: &str) -> Option<String> {
    context
        .baggage()
        .get(key)
        .map(|value| value.as_str().to_owned())
}

#[test]
fn set_request_baggage_adds_new_entries() {
    let context = set_request_baggage(
        &values(&[
            ("litellm.team.id", "team-1"),
            ("litellm.api_key.hash", "hash-1"),
        ]),
        None,
    );
    assert_eq!(
        baggage_entry(&context, "litellm.team.id"),
        Some("team-1".to_owned())
    );
    assert_eq!(
        baggage_entry(&context, "litellm.api_key.hash"),
        Some("hash-1".to_owned())
    );
}

#[test]
fn set_request_baggage_keeps_the_baggage_the_context_already_carries() {
    let base =
        Context::new().with_baggage([opentelemetry::KeyValue::new("litellm.team.id", "team-1")]);
    let context = set_request_baggage(&values(&[("litellm.end_user.id", "user-1")]), Some(&base));
    assert_eq!(
        baggage_entry(&context, "litellm.team.id"),
        Some("team-1".to_owned())
    );
    assert_eq!(
        baggage_entry(&context, "litellm.end_user.id"),
        Some("user-1".to_owned())
    );
}

#[test]
fn set_request_baggage_replaces_an_existing_key() {
    let base =
        Context::new().with_baggage([opentelemetry::KeyValue::new("litellm.team.id", "old-team")]);
    let context = set_request_baggage(&values(&[("litellm.team.id", "new-team")]), Some(&base));
    assert_eq!(
        baggage_entry(&context, "litellm.team.id"),
        Some("new-team".to_owned())
    );
}

#[test]
fn an_empty_mapping_leaves_the_baggage_as_it_was() {
    let base =
        Context::new().with_baggage([opentelemetry::KeyValue::new("litellm.team.id", "team-1")]);
    let context = set_request_baggage(&BTreeMap::new(), Some(&base));
    assert_eq!(
        baggage_entry(&context, "litellm.team.id"),
        Some("team-1".to_owned())
    );
}

#[test]
fn the_baggage_survives_a_context_derived_for_a_child_span() {
    let context = set_request_baggage(&values(&[("litellm.team.id", "team-1")]), None);
    let child = context.with_remote_span_context(SpanContext::new(
        TraceId::from(1_u128),
        SpanId::from(2_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    ));
    assert_eq!(
        baggage_entry(&child, "litellm.team.id"),
        Some("team-1".to_owned())
    );
    assert!(child.span().span_context().is_valid());
}

#[test]
fn a_value_over_the_baggage_byte_limit_is_not_stored() {
    let oversized = "x".repeat(8192);
    let context = set_request_baggage(
        &values(&[
            ("litellm.metadata.big", oversized.as_str()),
            ("litellm.team.id", "team-1"),
        ]),
        None,
    );
    assert_eq!(baggage_entry(&context, "litellm.metadata.big"), None);
    assert_eq!(
        baggage_entry(&context, "litellm.team.id"),
        Some("team-1".to_owned())
    );
}

#[test]
fn entries_past_the_sixty_fourth_are_not_stored() {
    let entries: BTreeMap<String, String> = (0_u32..65_u32)
        .map(|index| (format!("litellm.metadata.k{index:02}"), "v".to_owned()))
        .collect();
    let context = set_request_baggage(&entries, None);
    assert_eq!(context.baggage().len(), 64);
    assert_eq!(baggage_entry(&context, "litellm.metadata.k64"), None);
}

#[test]
fn a_key_with_a_separator_character_is_not_stored() {
    let context = set_request_baggage(&values(&[("litellm/team", "team-1")]), None);
    assert_eq!(baggage_entry(&context, "litellm/team"), None);
}
