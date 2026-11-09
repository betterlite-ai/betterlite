#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};

use crate::otel::model::metadata::wire::callback::{Callback, Params};
use crate::otel::model::metadata::wire::decode;
use crate::otel::model::metadata::wire::payload::Metadata;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::baggage_context::set_request_baggage;
use crate::otel::plumbing::context::parent_span::{
    ParentSpanSources, parent_otel_span_from_kwargs, span_context_from_kwargs,
    span_context_from_value,
};

const METADATA_TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const HEADER_TRACE: &str = "11111111111111111111111111111111";
const TOP_LEVEL_TRACE: &str = "22222222222222222222222222222222";

fn callback(text: &str) -> Callback {
    decode::<Callback>(text).expect("callback")
}

fn metadata(text: &str) -> Metadata {
    decode::<Metadata>(text).expect("metadata")
}

fn params(text: &str) -> Params {
    decode::<Params>(text).expect("params")
}

fn traceparent(trace: &str) -> String {
    format!("00-{trace}-b7ad6b7169203331-01")
}

fn ambient(trace_id: u128) -> Context {
    Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from(trace_id),
        SpanId::from(8_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    ))
}

#[test]
fn the_metadata_parent_wins_over_the_traceparent_header() {
    let callback = callback(&format!(
        r#"{{"litellm_params": {{
            "metadata": {{"litellm_parent_otel_span": "{}"}},
            "proxy_server_request": {{"headers": {{"traceparent": "{}"}}}}
        }}}}"#,
        traceparent(METADATA_TRACE),
        traceparent(HEADER_TRACE)
    ));
    let parent = span_context_from_kwargs(&callback);
    let context = parent.context.expect("context");
    assert_eq!(
        context.span().span_context().trace_id().to_string(),
        METADATA_TRACE
    );
    assert!(parent.span.is_none());
}

#[test]
fn the_metadata_parent_keeps_the_ambient_baggage() {
    let _ambient = set_request_baggage(
        &BTreeMap::from([("litellm.team.id".to_owned(), "team-1".to_owned())]),
        None,
    )
    .attach();
    let callback = callback(&format!(
        r#"{{"litellm_params": {{"metadata": {{"litellm_parent_otel_span": "{}"}}}}}}"#,
        traceparent(METADATA_TRACE)
    ));
    let context = span_context_from_kwargs(&callback)
        .context
        .expect("context");
    assert_eq!(
        context.span().span_context().trace_id().to_string(),
        METADATA_TRACE
    );
    assert_eq!(
        context
            .baggage()
            .get("litellm.team.id")
            .map(|value| value.as_str().to_owned()),
        Some("team-1".to_owned())
    );
}

#[test]
fn litellm_metadata_is_checked_after_metadata() {
    let callback = callback(&format!(
        r#"{{"litellm_params": {{"litellm_metadata": {{"litellm_parent_otel_span": "{}"}}}}}}"#,
        traceparent(METADATA_TRACE)
    ));
    let parent = span_context_from_kwargs(&callback);
    let context = parent.context.expect("context");
    assert_eq!(
        context.span().span_context().trace_id().to_string(),
        METADATA_TRACE
    );
}

#[test]
fn the_traceparent_header_is_the_parent_without_metadata() {
    let callback = callback(&format!(
        r#"{{"litellm_params": {{"proxy_server_request": {{"headers": {{"traceparent": "{}"}}}}}}}}"#,
        traceparent(HEADER_TRACE)
    ));
    let parent = span_context_from_kwargs(&callback);
    let context = parent.context.expect("context");
    assert_eq!(
        context.span().span_context().trace_id().to_string(),
        HEADER_TRACE
    );
    assert!(parent.span.is_none());
}

#[test]
fn the_active_span_is_returned_as_the_default_span() {
    let _guard = ambient(5).attach();
    let parent = span_context_from_kwargs(&callback("{}"));
    let context = parent.context.expect("context");
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(5));
    assert_eq!(
        parent.span.map(|span| span.trace_id()),
        Some(TraceId::from(5))
    );
}

#[test]
fn with_no_parent_anywhere_the_result_is_empty() {
    let parent = span_context_from_kwargs(&callback("{}"));
    assert!(parent.context.is_none());
    assert!(parent.span.is_none());
}

#[test]
fn the_kwargs_parent_comes_from_metadata_first() {
    let from_metadata = parent_otel_span_from_kwargs(&ParentSpanSources {
        metadata: Some(&metadata(&format!(
            r#"{{"litellm_parent_otel_span": "{}"}}"#,
            traceparent(METADATA_TRACE)
        ))),
        params: Some(&params(&format!(
            r#"{{"metadata": {{"litellm_parent_otel_span": "{}"}}}}"#,
            traceparent(HEADER_TRACE)
        ))),
        span: Some(&OtelModelValue::String(traceparent(TOP_LEVEL_TRACE))),
    });
    assert_eq!(
        from_metadata.map(|span| span.trace_id().to_string()),
        Some(METADATA_TRACE.to_owned())
    );
}

#[test]
fn the_kwargs_parent_falls_back_to_litellm_params_metadata() {
    let from_params = parent_otel_span_from_kwargs(&ParentSpanSources {
        metadata: None,
        params: Some(&params(&format!(
            r#"{{"metadata": {{"litellm_parent_otel_span": "{}"}}}}"#,
            traceparent(HEADER_TRACE)
        ))),
        span: None,
    });
    assert_eq!(
        from_params.map(|span| span.trace_id().to_string()),
        Some(HEADER_TRACE.to_owned())
    );
}

#[test]
fn the_kwargs_parent_falls_back_to_the_top_level_value_and_then_to_none() {
    let from_value = parent_otel_span_from_kwargs(&ParentSpanSources {
        metadata: None,
        params: None,
        span: Some(&OtelModelValue::String(traceparent(TOP_LEVEL_TRACE))),
    });
    assert_eq!(
        from_value.map(|span| span.trace_id().to_string()),
        Some(TOP_LEVEL_TRACE.to_owned())
    );
    assert!(
        parent_otel_span_from_kwargs(&ParentSpanSources {
            metadata: None,
            params: None,
            span: None,
        })
        .is_none()
    );
}

#[test]
fn a_present_null_parent_does_not_fall_through_to_the_next_place() {
    let null_metadata = metadata(r#"{"litellm_parent_otel_span": null}"#);
    let valid_params = params(&format!(
        r#"{{"metadata": {{"litellm_parent_otel_span": "{}"}}}}"#,
        traceparent(HEADER_TRACE)
    ));
    assert!(
        parent_otel_span_from_kwargs(&ParentSpanSources {
            metadata: Some(&null_metadata),
            params: Some(&valid_params),
            span: None,
        })
        .is_none()
    );
}

#[test]
fn an_object_carrier_is_read_as_a_traceparent() {
    let value = OtelModelValue::Object(BTreeMap::from([(
        "traceparent".to_owned(),
        OtelModelValue::String(traceparent(METADATA_TRACE)),
    )]));
    let span = span_context_from_value(&value).expect("span");
    assert_eq!(span.trace_id().to_string(), METADATA_TRACE);
}

#[test]
fn a_non_carrier_value_is_not_a_parent() {
    assert!(span_context_from_value(&OtelModelValue::Bool(true)).is_none());
    assert!(span_context_from_value(&OtelModelValue::Null(())).is_none());
    assert!(
        span_context_from_value(&OtelModelValue::String("not-a-traceparent".to_owned())).is_none()
    );
}
