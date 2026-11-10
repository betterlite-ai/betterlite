#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, KeyValue, StringValue};
use rstest::rstest;

use crate::otel::plumbing::context::post_response::{
    ServiceSpanTiming, resolve_parent_context, resolve_service_span_context,
};

fn span_context(trace_id: u128, span_id: u64) -> SpanContext {
    SpanContext::new(
        TraceId::from(trace_id),
        SpanId::from(span_id),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

fn moment(seconds: u64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

fn timing(
    end_time: Option<SystemTime>,
    parent_end_time: Option<SystemTime>,
    post_response: bool,
) -> ServiceSpanTiming {
    ServiceSpanTiming {
        end_time,
        parent_end_time,
        post_response,
    }
}

#[test]
fn work_that_ended_after_its_parent_links_back_instead_of_stretching_the_trace() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (context, links) =
        resolve_service_span_context(None, &timing(moment(20), moment(10), false));
    assert!(!context.span().span_context().is_valid());
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].span_context.trace_id(), TraceId::from(1));
    assert_eq!(links[0].span_context.span_id(), SpanId::from(2_u64));
}

#[test]
fn work_that_ended_before_its_parent_stays_nested() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (context, links) =
        resolve_service_span_context(None, &timing(moment(5), moment(10), false));
    assert_eq!(context.span().span_context().span_id(), SpanId::from(2_u64));
    assert!(links.is_empty());
}

#[test]
fn the_phase_marker_links_even_while_the_parent_is_still_open() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (context, links) = resolve_service_span_context(None, &timing(None, None, true));
    assert!(!context.span().span_context().is_valid());
    assert_eq!(links.len(), 1);
}

#[test]
fn an_unknown_end_time_after_a_finished_parent_links() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (_context, links) = resolve_service_span_context(None, &timing(None, moment(10), false));
    assert_eq!(links.len(), 1);
}

#[test]
fn a_parent_that_never_ended_keeps_a_request_phase_span_nested() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (context, links) = resolve_service_span_context(None, &timing(moment(20), None, false));
    assert_eq!(context.span().span_context().span_id(), SpanId::from(2_u64));
    assert!(links.is_empty());
}

#[test]
fn an_unsampled_parent_stays_the_parent_and_gets_no_link() {
    let unsampled = SpanContext::new(
        TraceId::from(1),
        SpanId::from(2),
        TraceFlags::default(),
        false,
        TraceState::default(),
    );
    let ambient = Context::new().with_remote_span_context(unsampled);
    let _guard = ambient.attach();
    let (context, links) = resolve_service_span_context(None, &timing(None, None, true));
    assert_eq!(context.span().span_context().span_id(), SpanId::from(2_u64));
    assert!(links.is_empty());
}

// the extracted-carrier shape a kwargs parent takes in betterlite
#[test]
fn a_carrier_parent_reaches_the_service_span_without_a_link() {
    let carrier = SpanContext::new(
        TraceId::from(1),
        SpanId::from(2),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let (context, links) = resolve_service_span_context(Some(&carrier), &timing(None, None, true));
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(1));
    assert!(links.is_empty());
}

#[test]
fn without_a_known_parent_the_timing_is_ignored() {
    let (context, links) =
        resolve_service_span_context(None, &timing(moment(20), moment(10), false));
    assert!(!context.span().span_context().is_valid());
    assert!(links.is_empty());
}

#[test]
fn baggage_survives_the_post_response_context() {
    let ambient = Context::new()
        .with_baggage([KeyValue::new("litellm.team.id", "team-1")])
        .with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (context, links) = resolve_service_span_context(None, &timing(None, None, true));
    assert_eq!(links.len(), 1);
    assert_eq!(
        context
            .baggage()
            .get("litellm.team.id")
            .map(StringValue::as_str),
        Some("team-1")
    );
}

#[test]
fn the_threaded_span_is_the_parent_when_nothing_is_ambient() {
    let threaded = span_context(3, 4);
    let context = resolve_parent_context(Some(&threaded));
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(3));
}

#[test]
fn the_ambient_span_wins_over_the_threaded_one() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let threaded = span_context(3, 4);
    let context = resolve_parent_context(Some(&threaded));
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(1));
}

#[test]
fn an_invalid_threaded_span_is_not_a_parent() {
    let invalid = SpanContext::NONE;
    let context = resolve_parent_context(Some(&invalid));
    assert!(!context.span().span_context().is_valid());
}

#[rstest]
#[case::equal(moment(10), moment(10), false)]
#[case::later(moment(11), moment(10), true)]
fn only_strictly_later_work_links_back(
    #[case] end: Option<SystemTime>,
    #[case] parent_end: Option<SystemTime>,
    #[case] expected: bool,
) {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let (_context, links) = resolve_service_span_context(None, &timing(end, parent_end, false));
    assert_eq!(!links.is_empty(), expected);
}
