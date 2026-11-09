#![cfg(test)]

use std::time::{Duration, UNIX_EPOCH};

use chrono::{TimeZone as _, Utc};
use opentelemetry::trace::{
    Span as _, SpanId, TraceContextExt as _, TraceId, Tracer as _, TracerProvider as _,
};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::trace::{
    IdGenerator, InMemorySpanExporter, RandomIdGenerator, SdkTracerProvider, Span as SdkSpan,
    SpanData,
};
use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    ChildSpanStart, GenerationStart, LangfuseObservation, parent_context, start_child_span,
    start_generation,
};
use crate::langfuse_observations::identity::Timestamp;
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;

#[derive(Clone, Copy, Debug)]
enum Check {
    FreshTraceHasNoParent,
    ExplicitParentIsRemote,
    ExistingTraceWithoutParentUsesTheGenerator,
    GenerationUsesRequestedIds,
    GenerationKeepsPublicFlag,
    GenerationLinksAnExplicitParent,
    GenerationLinksTheSyntheticParent,
    ChildSpanInheritsParentPublic,
    ChildSpanWithoutPublicAddsNoFlag,
    IdReadsTheSpanContext,
    EndConvertsSecondsToNanos,
    EndConvertsDatetimeToNanos,
    StartTimeIsSetFromSeconds,
}

#[derive(Clone, Copy, Debug)]
struct FixedGenerator {
    span_id: SpanId,
}

impl IdGenerator for FixedGenerator {
    fn new_trace_id(&self) -> TraceId {
        TraceId::INVALID
    }

    fn new_span_id(&self) -> SpanId {
        self.span_id
    }
}

fn trace(text: &str) -> TraceId {
    TraceId::from_hex(text).unwrap()
}

fn span(text: &str) -> SpanId {
    SpanId::from_hex(text).unwrap()
}

fn provider() -> (SdkTracerProvider, InMemorySpanExporter) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .with_id_generator(RequestedIdGenerator::default())
        .build();
    (provider, exporter)
}

fn single(exporter: &InMemorySpanExporter) -> SpanData {
    let mut spans = exporter.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 1);
    spans.remove(0)
}

fn generation(
    start: GenerationStart,
) -> (
    SdkTracerProvider,
    InMemorySpanExporter,
    LangfuseObservation<SdkSpan>,
) {
    let (provider, exporter) = provider();
    let tracer = provider.tracer("langfuse-observations");
    let observation = start_generation(&tracer, &RandomIdGenerator::default(), start);
    (provider, exporter, observation)
}

fn generation_start(public: Option<bool>) -> GenerationStart {
    GenerationStart {
        trace_id: trace("0102030405060708090a0b0c0d0e0f10"),
        parent_observation_id: None,
        existing_trace: false,
        observation_id: Some(span("0102030405060708")),
        name: "generation".to_owned(),
        start_time: None,
        public,
        attributes: Vec::new(),
    }
}

fn has_public_attribute(data: &SpanData, expected: bool) -> bool {
    data.attributes.iter().any(|pair| {
        pair.key.as_str() == "langfuse.trace.public" && pair.value == Value::Bool(expected)
    })
}

#[rstest]
#[case::fresh_trace_has_no_parent(Check::FreshTraceHasNoParent)]
#[case::explicit_parent_is_remote(Check::ExplicitParentIsRemote)]
#[case::existing_trace_without_parent_uses_the_generator(
    Check::ExistingTraceWithoutParentUsesTheGenerator
)]
#[case::generation_uses_requested_ids(Check::GenerationUsesRequestedIds)]
#[case::generation_keeps_public_flag(Check::GenerationKeepsPublicFlag)]
#[case::generation_links_an_explicit_parent(Check::GenerationLinksAnExplicitParent)]
#[case::generation_links_the_synthetic_parent(Check::GenerationLinksTheSyntheticParent)]
#[case::child_span_inherits_parent_public(Check::ChildSpanInheritsParentPublic)]
#[case::child_span_without_public_adds_no_flag(Check::ChildSpanWithoutPublicAddsNoFlag)]
#[case::id_reads_the_span_context(Check::IdReadsTheSpanContext)]
#[case::end_converts_seconds_to_nanos(Check::EndConvertsSecondsToNanos)]
#[case::end_converts_datetime_to_nanos(Check::EndConvertsDatetimeToNanos)]
#[case::start_time_is_set_from_seconds(Check::StartTimeIsSetFromSeconds)]
fn observation_attributes_tests_behavior(#[case] check: Check) {
    match check {
        Check::FreshTraceHasNoParent => {
            let generator = FixedGenerator {
                span_id: span("0000000000000002"),
            };
            let context = parent_context(
                trace("0102030405060708090a0b0c0d0e0f10"),
                None,
                false,
                &generator,
            );
            assert!(!context.has_active_span());
            assert!(!context.span().span_context().is_valid());
        }
        Check::ExplicitParentIsRemote => {
            let generator = FixedGenerator {
                span_id: span("0000000000000002"),
            };
            let parent = span("0000000000000005");
            let context = parent_context(
                trace("0102030405060708090a0b0c0d0e0f10"),
                Some(parent),
                true,
                &generator,
            );
            let span_context = context.span().span_context().clone();
            assert!(span_context.is_valid());
            assert_eq!(
                span_context.trace_id(),
                trace("0102030405060708090a0b0c0d0e0f10")
            );
            assert_eq!(span_context.span_id(), parent);
            assert!(span_context.is_remote());
            assert!(span_context.is_sampled());
        }
        Check::ExistingTraceWithoutParentUsesTheGenerator => {
            let generated = span("0000000000000009");
            let context = parent_context(
                trace("0102030405060708090a0b0c0d0e0f10"),
                None,
                true,
                &FixedGenerator { span_id: generated },
            );
            let span_context = context.span().span_context().clone();
            assert!(span_context.is_valid());
            assert_eq!(span_context.span_id(), generated);
            assert!(span_context.is_remote());
        }
        Check::GenerationUsesRequestedIds => {
            let (_provider, exporter, mut observation) = generation(generation_start(None));
            assert_eq!(observation.id, "0102030405060708");
            observation.end(None);
            let data = single(&exporter);
            assert_eq!(
                data.span_context.trace_id(),
                trace("0102030405060708090a0b0c0d0e0f10")
            );
            assert_eq!(data.span_context.span_id(), span("0102030405060708"));
            assert_eq!(data.name, "generation");
        }
        Check::GenerationKeepsPublicFlag => {
            let (_bare_provider, bare_exporter, mut bare) =
                generation(generation_start(Some(true)));
            assert_eq!(bare.public, Some(true));
            bare.end(None);
            assert!(!has_public_attribute(&single(&bare_exporter), true));
            let mut start = generation_start(Some(true));
            start.attributes = vec![KeyValue::new("langfuse.trace.public", true)];
            let (_provider, exporter, mut observation) = generation(start);
            observation.end(None);
            let data = single(&exporter);
            assert!(has_public_attribute(&data, true));
            assert_eq!(data.attributes.len(), 1);
        }
        Check::GenerationLinksAnExplicitParent => {
            let parent = span("0000000000000005");
            let mut start = generation_start(None);
            start.parent_observation_id = Some(parent);
            let (_provider, exporter, mut observation) = generation(start);
            observation.end(None);
            let data = single(&exporter);
            assert_eq!(data.parent_span_id, parent);
            assert!(data.parent_span_is_remote);
        }
        Check::GenerationLinksTheSyntheticParent => {
            let synthetic = span("0000000000000009");
            let mut start = generation_start(None);
            start.existing_trace = true;
            let (provider, exporter) = provider();
            let tracer = provider.tracer("langfuse-observations");
            let mut observation =
                start_generation(&tracer, &FixedGenerator { span_id: synthetic }, start);
            observation.end(None);
            let data = single(&exporter);
            assert_eq!(data.parent_span_id, synthetic);
            assert!(data.parent_span_is_remote);
        }
        Check::ChildSpanInheritsParentPublic => {
            let (provider, exporter) = provider();
            let tracer = provider.tracer("langfuse-observations");
            let parent = start_generation(
                &tracer,
                &RandomIdGenerator::default(),
                generation_start(Some(true)),
            );
            let mut child = start_child_span(
                &tracer,
                &parent,
                ChildSpanStart {
                    name: "guardrail".to_owned(),
                    start_time: None,
                    attributes: Vec::new(),
                },
            );
            assert_eq!(child.public, Some(true));
            let parent_span_id = span("0102030405060708");
            child.end(None);
            let data = single(&exporter);
            assert_eq!(data.name, "guardrail");
            assert_eq!(data.parent_span_id, parent_span_id);
            assert!(has_public_attribute(&data, true));
        }
        Check::ChildSpanWithoutPublicAddsNoFlag => {
            let (provider, exporter) = provider();
            let tracer = provider.tracer("langfuse-observations");
            let parent = start_generation(
                &tracer,
                &RandomIdGenerator::default(),
                generation_start(None),
            );
            let mut child = start_child_span(
                &tracer,
                &parent,
                ChildSpanStart {
                    name: "guardrail".to_owned(),
                    start_time: None,
                    attributes: Vec::new(),
                },
            );
            assert_eq!(child.public, None);
            child.end(None);
            let data = single(&exporter);
            assert!(!has_public_attribute(&data, true));
            assert!(!has_public_attribute(&data, false));
        }
        Check::IdReadsTheSpanContext => {
            let (provider, _exporter) = provider();
            let span = provider.tracer("langfuse-observations").start("bare");
            let observation = LangfuseObservation::new(span, None);
            assert_eq!(
                observation.id,
                observation.span.span_context().span_id().to_string()
            );
        }
        Check::EndConvertsSecondsToNanos => {
            let (_provider, exporter, mut observation) = generation(generation_start(None));
            observation.end(Some(Timestamp::Seconds(Decimal::new(12_345, 1))));
            let data = single(&exporter);
            assert_eq!(
                data.end_time,
                UNIX_EPOCH
                    .checked_add(Duration::from_millis(1_234_500))
                    .unwrap()
            );
        }
        Check::EndConvertsDatetimeToNanos => {
            let (_provider, exporter, mut observation) = generation(generation_start(None));
            let datetime = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            observation.end(Some(Timestamp::DateTime(datetime)));
            let data = single(&exporter);
            assert_eq!(
                data.end_time,
                UNIX_EPOCH
                    .checked_add(Duration::from_hours(490_896))
                    .unwrap()
            );
        }
        Check::StartTimeIsSetFromSeconds => {
            let mut start = generation_start(None);
            start.start_time = Some(Timestamp::Seconds(Decimal::new(12_345, 1)));
            let (_provider, exporter, mut observation) = generation(start);
            observation.end(None);
            let data = single(&exporter);
            assert_eq!(
                data.start_time,
                UNIX_EPOCH
                    .checked_add(Duration::from_millis(1_234_500))
                    .unwrap()
            );
        }
    }
}
