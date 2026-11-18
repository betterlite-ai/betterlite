use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status};
use std::sync::Arc;
use std::time::SystemTime;

use crate::support::CountingFailureCounter;
use crate::support::{END, Rig, START, attach, call, in_memory_config, instant, simple_kwargs};
use betterlite_logging::otel::legacy_lifecycle::state::RequestDedupe;
use betterlite_logging::otel::legacy_metrics::events::SemanticEventEmitter;
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::legacy_metrics::recorder::LegacyMetricRecorder;
use betterlite_logging::otel::legacy_policy::dedupe::ScopeValue;
use betterlite_logging::otel::legacy_policy::error_recording::RecordedException;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::events::Clock;
use betterlite_logging::otel::vocabulary::semconv::OTELSemconvCategory;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use chrono::FixedOffset;
use opentelemetry::logs::LoggerProvider as _;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

struct NoCallbackSettings;

impl CallbackSettingsView for NoCallbackSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[test]
fn legacy_lifecycle_success_without_a_parent_creates_the_primary_and_raw_spans() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-1");
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    let primary = rig.span("litellm_request");
    assert_eq!(primary.status, Status::Ok);
    assert_eq!(
        crate::support::text_attribute(&primary, "gen_ai.request.model"),
        "gpt-4o"
    );
    assert_eq!(primary.start_time, instant(START));
    assert_eq!(primary.end_time, instant(END));
    let raw = rig.span("raw_gen_ai_request");
    assert_eq!(raw.parent_span_id, primary.span_context.span_id());
    assert_eq!(raw.status, Status::Ok);
    assert_eq!(raw.start_time, instant(START));
    assert_eq!(raw.end_time, instant(END));
}

#[test]
fn legacy_lifecycle_raw_request_uses_the_generation_name() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-generation-name");
    let Some(betterlite_logging::otel::model::utils::coercion::OtelModelValue::Object(params)) =
        kwargs.get_mut("litellm_params")
    else {
        panic!("litellm_params must be an object");
    };
    let Some(betterlite_logging::otel::model::utils::coercion::OtelModelValue::Object(metadata)) =
        params.get_mut("metadata")
    else {
        panic!("metadata must be an object");
    };
    metadata.insert(
        "generation_name".to_owned(),
        betterlite_logging::otel::model::utils::coercion::OtelModelValue::String(
            "generation-name".to_owned(),
        ),
    );

    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");

    assert!(rig.names().contains(&"generation-name".to_owned()));
}

#[test]
fn legacy_lifecycle_skips_the_raw_request_span_in_semconv_mode() {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config
        .semconv_stability_opt_in
        .insert(OTELSemconvCategory::GenAiLatestExperimental);
    let lifecycle = rig.with_options(config, None, None, false);
    let kwargs = simple_kwargs("call-semconv");

    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");

    let names = rig.names();
    assert_eq!(names.len(), 1);
    assert_ne!(names[0], "raw_gen_ai_request");
}

#[test]
fn legacy_lifecycle_success_with_an_ambient_parent_keeps_the_hierarchy_shallow() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let parent = rig.parent(&lifecycle, "upstream");
    let kwargs = simple_kwargs("call-2");
    let context = attach(&parent.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    drop(guard);
    assert_eq!(rig.names(), vec!["raw_gen_ai_request"]);
    assert_eq!(
        rig.span("raw_gen_ai_request").parent_span_id,
        parent.context.span_id()
    );
    let data = parent
        .lock()
        .exported_data()
        .expect("the parent stays open");
    assert!(parent.lock().is_recording());
    assert_eq!(data.status, Status::Ok);
    assert!(
        !data.attributes.is_empty(),
        "the shallow path stamps the parent span"
    );
}

#[test]
fn legacy_lifecycle_creates_the_primary_span_when_the_request_span_flag_is_on() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(in_memory_config(), None, None, true);
    let parent = rig.parent(&lifecycle, "upstream");
    let kwargs = simple_kwargs("call-3");
    let context = attach(&parent.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    drop(guard);
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    let primary = rig.span("litellm_request");
    assert_eq!(primary.parent_span_id, parent.context.span_id());
}

#[test]
fn legacy_lifecycle_failure_without_a_parent_creates_the_error_span() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-4");
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("failure lifecycle");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "litellm_request");
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert_eq!(spans[0].end_time, instant(END));
}

#[test]
fn legacy_lifecycle_failure_records_the_exception_event() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-exception");
    let exception = RecordedException {
        type_name: "ProviderError".to_owned(),
        message: "request failed".to_owned(),
        stacktrace: String::new(),
    };
    let mut lifecycle_call = call(&kwargs);
    lifecycle_call.exception = Some(&exception);

    lifecycle
        .handle_failure(&lifecycle_call)
        .expect("failure lifecycle");

    let span = rig.span("litellm_request");
    assert!(span.events.iter().any(|event| {
        event.name.as_ref() == "exception"
            && event.attributes.iter().any(|attribute| {
                attribute.key.as_str() == "exception.message"
                    && attribute.value == opentelemetry::Value::from("request failed")
            })
    }));
}

#[test]
fn legacy_lifecycle_failure_with_an_ambient_parent_stamps_the_parent() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let parent = rig.parent(&lifecycle, "upstream");
    let kwargs = simple_kwargs("call-5");
    let context = attach(&parent.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("failure lifecycle");
    drop(guard);
    assert!(rig.spans().is_empty(), "no new span is created");
    let data = parent
        .lock()
        .exported_data()
        .expect("the parent stays open");
    assert!(matches!(data.status, Status::Error { .. }));
    assert!(!data.attributes.is_empty());
    assert!(
        parent.lock().is_recording(),
        "only a proxy span is closed by the failure hook"
    );
}

#[test]
fn legacy_lifecycle_success_events_dedupe_the_sync_and_async_fire() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-6");
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("first success");
    lifecycle
        .handle_success(&call(&kwargs))
        .expect("duplicate success");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
}

#[test]
fn legacy_lifecycle_failure_events_dedupe_the_sync_and_async_fire() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let kwargs = simple_kwargs("call-7");
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("first failure");
    lifecycle
        .handle_failure(&call(&kwargs))
        .expect("duplicate failure");
    assert_eq!(rig.names(), vec!["litellm_request"]);
}

#[test]
fn legacy_lifecycle_keeps_distinct_request_ids_independent() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let first = simple_kwargs("call-first");
    let second = simple_kwargs("call-second");

    lifecycle
        .handle_success(&call(&first))
        .expect("first success");
    lifecycle
        .handle_success(&call(&second))
        .expect("second success");

    assert_eq!(
        rig.names(),
        vec![
            "litellm_request",
            "raw_gen_ai_request",
            "litellm_request",
            "raw_gen_ai_request"
        ]
    );
}

#[test]
fn legacy_lifecycle_evicts_the_oldest_dedupe_request_at_capacity() {
    let dedupe = RequestDedupe::new("OpenTelemetry".to_owned());
    let scope = [ScopeValue::Text("success".to_owned())];
    assert!((0..10_000).all(|index| {
        let key = format!("call-{index}");
        dedupe.emit_once(Some(key.as_str()), &scope)
    }));

    assert!(
        !dedupe.emit_once(Some("call-5"), &scope),
        "a repeat emits nothing at capacity"
    );
    assert!(
        !dedupe.emit_once(Some("call-0"), &scope),
        "the oldest request stays tracked"
    );
    assert!(
        dedupe.emit_once(Some("call-10000"), &scope),
        "a new request at capacity evicts the oldest"
    );
    assert!(
        dedupe.emit_once(Some("call-0"), &scope),
        "the evicted request emits again"
    );
}

#[test]
fn legacy_lifecycle_keeps_dedupe_state_below_capacity() {
    let dedupe = RequestDedupe::new("OpenTelemetry".to_owned());
    let scope = [ScopeValue::Text("success".to_owned())];
    assert!(dedupe.emit_once(Some("call-a"), &scope));
    assert!(dedupe.emit_once(Some("call-b"), &scope));
    assert!(
        !dedupe.emit_once(Some("call-a"), &scope),
        "a second request does not evict the first below capacity"
    );
}

#[test]
fn legacy_lifecycle_uses_the_clock_for_missing_span_times() {
    let rig = Rig::new();
    let clock_time = instant(START + 12.25);
    let lifecycle = rig.with_services(
        in_memory_config(),
        None,
        None,
        false,
        Box::new(FixedClock(clock_time)),
        None,
        None,
        Vec::new(),
    );
    let kwargs = simple_kwargs("call-missing-times");
    let mut lifecycle_call = call(&kwargs);
    lifecycle_call.start_time = SpanTime::Missing;
    lifecycle_call.end_time = SpanTime::Missing;

    lifecycle
        .handle_success(&lifecycle_call)
        .expect("success lifecycle");

    let primary = rig.span("litellm_request");
    assert_eq!(primary.start_time, clock_time);
    assert_eq!(primary.end_time, clock_time);
}

#[test]
fn legacy_lifecycle_reports_attribute_failures_to_callback_counter() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let rig = Rig::new();
    let counter = Arc::new(CountingFailureCounter(AtomicUsize::new(0)));
    let lifecycle = rig.with_services(
        in_memory_config(),
        None,
        None,
        false,
        Box::new(FixedClock(instant(START))),
        None,
        None,
        vec![counter.clone()],
    );
    let mut kwargs = simple_kwargs("call-missing-standard-log");
    kwargs.remove("standard_logging_object");

    lifecycle
        .handle_success(&call(&kwargs))
        .expect("lifecycle handles attribute failure");

    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
}

#[test]
fn legacy_lifecycle_records_metrics_for_success() {
    let rig = Rig::new();
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let config = in_memory_config();
    let recorder = LegacyMetricRecorder::new(
        create_instruments(&provider.meter("legacy-lifecycle-test")),
        &config,
        None,
        Arc::new(NoCallbackSettings),
        FixedOffset::east_opt(0).expect("UTC offset"),
    )
    .expect("metric recorder");
    let lifecycle = rig.with_services(
        config,
        None,
        None,
        false,
        Box::new(FixedClock(instant(START))),
        Some(recorder),
        None,
        Vec::new(),
    );
    let kwargs = simple_kwargs("call-metrics");

    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    provider.force_flush().expect("flush metrics");
    let metrics = exporter.get_finished_metrics().expect("metrics");
    let duration = metrics
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .find(|metric| metric.name() == "gen_ai.client.operation.duration")
        .expect("operation duration metric");
    let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration.data() else {
        panic!("operation duration must be a histogram");
    };
    assert!(histogram.data_points().any(|point| point.sum() == 1.5));
}

#[test]
fn legacy_lifecycle_emits_semantic_logs_on_success() {
    let rig = Rig::new();
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let events = SemanticEventEmitter {
        event_logger: provider.logger("legacy-lifecycle-test"),
        clock: Box::new(FixedClock(instant(END))),
        enable_events: true,
        semconv_latest: false,
    };
    let lifecycle = rig.with_services(
        in_memory_config(),
        None,
        None,
        false,
        Box::new(FixedClock(instant(START))),
        None,
        Some(events),
        Vec::new(),
    );
    let kwargs = simple_kwargs("call-events");

    lifecycle
        .handle_success(&call(&kwargs))
        .expect("success lifecycle");
    provider.force_flush().expect("flush logs");

    assert!(
        !exporter
            .get_emitted_logs()
            .expect("emitted logs")
            .is_empty()
    );
}

#[rstest::rstest]
#[case::success(false)]
#[case::failure(true)]
fn legacy_lifecycle_ignores_parent_when_configured(#[case] failure: bool) {
    let rig = Rig::new();
    let mut config = in_memory_config();
    config.ignore_context_propagation = true;
    let lifecycle = rig.with_options(config, None, None, false);
    let parent = rig.parent(&lifecycle, "upstream");
    let mut kwargs = simple_kwargs("ignored-parent");
    crate::support::carry_in(&mut kwargs, "metadata", &parent);
    let context = attach(&parent.context);
    let guard = Context::attach(&context);

    if failure {
        lifecycle
            .handle_failure(&call(&kwargs))
            .expect("failure lifecycle");
    } else {
        lifecycle
            .handle_success(&call(&kwargs))
            .expect("success lifecycle");
    }
    drop(guard);

    let primary = rig.span("litellm_request");
    assert_eq!(
        primary.parent_span_id,
        opentelemetry::trace::SpanId::INVALID
    );
    assert_ne!(primary.span_context.trace_id(), parent.context.trace_id());
    assert!(parent.lock().is_recording());
}
