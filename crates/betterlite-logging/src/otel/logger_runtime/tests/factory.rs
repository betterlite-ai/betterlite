#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider, SimpleLogProcessor};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::callback_logger::CallbackLogger;
use crate::logging_pre_post::hooks::{ApiCallLogger as _, PreCallEvent};
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::content_projections::OutputResponse;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::error::LoggerRuntimeError;
use crate::otel::logger_runtime::factory::{build_otel_v2_logger, logger_class};
use crate::otel::logger_runtime::preset::{
    exports_nowhere, has_operator_exporter, is_credential_gated, is_gated, maybe_construct_otel_v2,
    only_the_gated_exporter,
};
use crate::otel::logger_runtime::types::{
    CallbackHooks, ConstructorSettings, InjectedProviders, LoggerClass,
};
use crate::otel::mappers::registry::MapperError;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::OperationException;
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::v2_callback::error::CallbackError;

use super::support::{
    Fixture, RootLog, callback_constructor, destination, finished, fixture, fixture_disabled,
    fixture_with_base, gated, lock, operator_exporter, placeholder, probe, provider_exporting,
    recording_root, settings, spec,
};

fn build_with_settings(
    fixture: &Fixture,
    config: Settings,
    name: &str,
    constructor_settings: ConstructorSettings,
) {
    build_otel_v2_logger(
        &fixture.runtime,
        config,
        Some(name),
        InjectedProviders::default(),
        constructor_settings,
    )
    .expect("built");
}

fn recorded_runtime(fixture: Fixture) -> (RootLog, LoggerRuntime) {
    let (log, roots) = recording_root();
    let runtime = LoggerRuntime {
        constructor: Arc::new(callback_constructor(roots)),
        ..fixture.runtime
    };
    (log, runtime)
}

fn record_failure(
    callback: &V2Callback,
    meter: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
) {
    callback
        .recorders()
        .0
        .expect("metric recorder")
        .record_failure(
            &MetadataMap::new(),
            Decimal::from(10_i64),
            Decimal::from(13_i64),
        )
        .expect("record");
    meter.force_flush().expect("meter flush");
    assert!(
        !exporter
            .get_finished_metrics()
            .expect("finished metrics")
            .is_empty()
    );
}

fn record_exception(callback: &V2Callback, exporter: &InMemoryLogExporter) {
    callback
        .recorders()
        .1
        .expect("event recorder")
        .record_operation_exception(&OperationException {
            span_context: &span_context(),
            error_type: "Boom",
            message: "kaboom",
            stack_trace: None,
            timestamp_ns: None,
        });
    assert_eq!(exporter.get_emitted_logs().expect("logs").len(), 1);
}

fn assert_root_log(log: &RootLog, expected: &[&str]) {
    assert_eq!(*lock(log), expected);
}

fn pre_call_details() -> Json {
    serde_json::from_str(
        r#"{
            "litellm_call_id": "call-1",
            "model": "gpt-4o",
            "litellm_params": {
                "metadata": {
                    "trace_name": "checkout",
                    "trace_user_id": "user-1",
                    "session_id": "session-1",
                    "tags": ["env:test", "beta"]
                }
            }
        }"#,
    )
    .expect("details")
}

fn add_carrier(fixture: &Fixture, in_memory: &mut Vec<Arc<dyn CallbackLogger>>, name: &str) {
    let carrier = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("carrier"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("carrier");
    in_memory.push(Arc::clone(&carrier.callback));
    fixture.trace.set_destinations(vec![destination(name)]);
}

fn span_context() -> SpanContext {
    SpanContext::new(
        TraceId::from(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10_u128),
        SpanId::from(0x1112_1314_1516_1718_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

#[test]
fn factory_provider_passthrough() {
    let (_log, runtime) = recorded_runtime(fixture());
    let mut config = settings(vec![]);
    config.enable_metrics = true;
    config.enable_events = true;
    let tracer_sink = InMemorySpanExporter::default();
    let tracer = provider_exporting(&tracer_sink);
    let metric_exporter = InMemoryMetricExporter::default();
    let meter = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(metric_exporter.clone()).build())
        .build();
    let log_exporter = InMemoryLogExporter::default();
    let logs = SdkLoggerProvider::builder()
        .with_log_processor(SimpleLogProcessor::new(log_exporter.clone()))
        .build();
    let logger = build_otel_v2_logger(
        &runtime,
        config,
        None,
        InjectedProviders {
            tracer_provider: Some(&tracer),
            logger_provider: Some(&logs),
            meter_provider: Some(&meter),
        },
        ConstructorSettings::default(),
    )
    .expect("built");
    // LiteLLM keeps the caller's callback_name, None here, and betterlite's str view of it is empty
    assert_eq!(logger.callback.callback_name(), "");
    probe(&logger.provider);
    assert_eq!(finished(&tracer_sink), 1);
    let Some(CallbackHooks::Generic(callback)) = &logger.hooks else {
        panic!("the generic class keeps its typed callback");
    };
    // each recorder emits through the provider the caller injected, never one the build replaces
    record_failure(callback, &meter, &metric_exporter);
    record_exception(callback, &log_exporter);
}

#[test]
fn factory_chooses_generic_class() {
    let fixture = fixture();
    build_with_settings(
        &fixture,
        settings(vec![]),
        "otel",
        ConstructorSettings::default(),
    );
    let records = fixture.constructor.records();
    assert_eq!(records[0].class, LoggerClass::Generic);
    assert_eq!(records[0].callback_name.as_deref(), Some("otel"));
    assert!(records[0].assembler.is_none());
    assert_eq!(records[0].config, settings(vec![]));
}

#[test]
fn factory_chooses_langfuse_class() {
    let fixture = fixture();
    let mut config = settings(vec![]);
    config.mapper_names = vec!["genai".to_owned(), "langfuse".to_owned()];
    build_with_settings(
        &fixture,
        config,
        "langfuse_otel",
        ConstructorSettings::default(),
    );
    let records = fixture.constructor.records();
    assert_eq!(records[0].class, LoggerClass::Langfuse);
    assert!(records[0].assembler.is_none());
}

#[test]
fn factory_chooses_langfuse_content_class() {
    let fixture = fixture();
    let mut config = settings(vec![]);
    config.mapper_names = vec!["langfuse".to_owned()];
    config.capture_message_content = CaptureMessageContent::SpanAndEvent;
    build_with_settings(
        &fixture,
        config,
        "langfuse_otel",
        ConstructorSettings::default(),
    );
    let records = fixture.constructor.records();
    assert_eq!(records[0].class, LoggerClass::LangfuseContent);
    let assembler = records[0].assembler.as_ref().expect("the assembler");
    assert!(Arc::ptr_eq(assembler, &fixture.assembler));
}

#[test]
fn factory_forwards_constructor_settings() {
    let fixture = fixture();
    let constructor_settings = ConstructorSettings {
        turn_off_message_logging: true,
        message_logging: false,
    };
    build_with_settings(&fixture, settings(vec![]), "otel", constructor_settings);
    let mut langfuse = settings(vec![]);
    langfuse.mapper_names = vec!["langfuse".to_owned()];
    build_with_settings(
        &fixture,
        langfuse.clone(),
        "langfuse_otel",
        constructor_settings,
    );
    langfuse.capture_message_content = CaptureMessageContent::SpanAndEvent;
    build_with_settings(&fixture, langfuse, "langfuse_otel", constructor_settings);
    let records = fixture.constructor.records();
    assert_eq!(
        records
            .iter()
            .map(|record| record.class)
            .collect::<Vec<_>>(),
        vec![
            LoggerClass::Generic,
            LoggerClass::Langfuse,
            LoggerClass::LangfuseContent
        ]
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.settings)
            .collect::<Vec<_>>(),
        vec![constructor_settings; 3]
    );
}

#[test]
fn constructor_settings_default_to_message_logging_on() {
    assert_eq!(
        ConstructorSettings::default(),
        ConstructorSettings {
            turn_off_message_logging: false,
            message_logging: true,
        }
    );
}

#[test]
fn logger_class_ignores_langfuse_otel() {
    let mut config = settings(vec![]);
    config.mapper_names = vec!["genai".to_owned(), "langfuse_otel".to_owned()];
    config.capture_message_content = CaptureMessageContent::SpanOnly;
    assert_eq!(logger_class(&config), LoggerClass::Generic);
}

#[test]
fn logger_class_content_needs_span_capture() {
    let mut config = settings(vec![]);
    config.mapper_names = vec!["langfuse".to_owned()];
    config.capture_message_content = CaptureMessageContent::EventOnly;
    assert_eq!(logger_class(&config), LoggerClass::Langfuse);
    config.capture_message_content = CaptureMessageContent::SpanOnly;
    assert_eq!(logger_class(&config), LoggerClass::LangfuseContent);
}

#[rstest]
#[case::absent_headers((None, true, true))]
#[case::empty_headers((Some(""), true, true))]
#[case::present_headers((Some("authorization=x"), true, false))]
#[case::header_optional((None, false, false))]
fn is_gated_table(#[case] case: (Option<&str>, bool, bool)) {
    let (headers, requires_headers, expected) = case;
    assert_eq!(
        is_gated(&spec("otlp_http", None, headers, requires_headers)),
        expected
    );
}

#[rstest]
#[case::empty(vec![], true)]
#[case::only_gated(vec![gated()], true)]
#[case::mixed(vec![gated(), operator_exporter()], false)]
#[case::only_operator(vec![operator_exporter()], false)]
fn exports_nowhere_table(#[case] exporters: Vec<ExporterSpec>, #[case] expected: bool) {
    assert_eq!(exports_nowhere(&settings(exporters)), expected);
}

#[rstest]
#[case::empty(vec![], false)]
#[case::gated(vec![gated()], true)]
#[case::operator(vec![operator_exporter()], false)]
fn is_credential_gated_table(#[case] exporters: Vec<ExporterSpec>, #[case] expected: bool) {
    assert_eq!(is_credential_gated(&settings(exporters)), expected);
}

#[rstest]
#[case::empty(vec![], false)]
#[case::placeholder_only(vec![placeholder()], false)]
#[case::gated_only(vec![gated()], false)]
#[case::operator(vec![operator_exporter()], true)]
fn has_operator_exporter_table(#[case] exporters: Vec<ExporterSpec>, #[case] expected: bool) {
    assert_eq!(has_operator_exporter(&settings(exporters)), expected);
}

#[test]
fn only_the_gated_exporter_keeps_a_copy() {
    let config = settings(vec![operator_exporter(), gated()]);
    let gated_only = only_the_gated_exporter(&config);
    assert_eq!(gated_only.exporters, vec![gated()]);
    assert_eq!(config.exporters.len(), 2);
    assert_eq!(gated_only.service_name, config.service_name);
}

#[test]
fn v2_disabled_returns_none() {
    let fixture = fixture_disabled();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome =
        maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory).expect("no failure");
    assert!(outcome.is_none());
    assert!(in_memory.is_empty());
    assert_eq!(fixture.constructor.records().len(), 0);
}

#[test]
fn unknown_callback_returns_none() {
    let fixture = fixture();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome =
        maybe_construct_otel_v2(&fixture.runtime, "otel", &mut in_memory).expect("no failure");
    assert!(outcome.is_none());
    assert_eq!(fixture.constructor.records().len(), 0);
}

#[test]
fn preset_failure_returns_none() {
    let fixture = fixture();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "langfuse_otel", &mut in_memory)
        .expect("preset failures are absorbed");
    assert!(outcome.is_none());
    assert_eq!(fixture.constructor.records().len(), 0);
}

#[test]
fn constructor_failure_propagates() {
    let fixture = fixture();
    fixture
        .environment
        .set("NEW_RELIC_LICENSE_KEY", "license-key");
    fixture
        .constructor
        .fail_with(CallbackError::Mapper(MapperError::Unknown {
            name: "boom".to_owned(),
            known: String::new(),
        }));
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let error = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .err()
        .expect("the constructor failure surfaces");
    assert!(matches!(
        &error,
        LoggerRuntimeError::Constructor(CallbackError::Mapper(MapperError::Unknown { name, .. }))
            if name == "boom"
    ));
}

#[test]
fn gated_preset_without_carrier_or_operator_is_skipped() {
    let fixture = fixture();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome =
        maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory).expect("no failure");
    assert!(outcome.is_none());
    assert_eq!(fixture.constructor.records().len(), 0);
}

#[test]
fn a_destination_alone_does_not_carry_credentials() {
    let fixture = fixture();
    fixture
        .trace
        .set_destinations(vec![destination("newrelic")]);
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome =
        maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory).expect("no failure");
    assert!(outcome.is_none());
    assert!(in_memory.is_empty());
    assert_eq!(fixture.constructor.records().len(), 0);
}

#[test]
fn gated_preset_without_carrier_uses_an_operator_exporter() {
    let fixture = fixture_with_base(settings(vec![placeholder(), operator_exporter()]));
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("built");
    let records = fixture.constructor.records();
    assert_eq!(records[0].config.exporters.len(), 3);
    assert_eq!(outcome.config, records[0].config);
    assert_eq!(in_memory.len(), 1);
}

#[test]
fn gated_preset_with_carrier_keeps_only_gated_exporters() {
    let fixture = fixture();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    add_carrier(&fixture, &mut in_memory, "newrelic");
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("built");
    let records = fixture.constructor.records();
    let built = records.last().expect("the built logger");
    assert_eq!(built.config.exporters.len(), 1);
    assert!(is_gated(&built.config.exporters[0]));
    assert!(exports_nowhere(&built.config));
    assert_eq!(outcome.config, built.config);
    assert_eq!(in_memory.len(), 2);
}

#[test]
fn gated_preset_with_carrier_drops_operator_exporters() {
    let fixture = fixture_with_base(settings(vec![operator_exporter()]));
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    add_carrier(&fixture, &mut in_memory, "newrelic");
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("built");
    let records = fixture.constructor.records();
    let built = records.last().expect("the built logger");
    // the gated New Relic exporter is retained as the preset built it, endpoint and owner included
    let newrelic = ExporterSpec {
        owner: Some(ExporterOwner::Newrelic),
        ..spec("otlp_http", Some("https://otlp.nr-data.net"), None, true)
    };
    assert_eq!(built.config.exporters, vec![newrelic]);
    assert_eq!(
        built.config.exporters[0].endpoint.as_deref(),
        Some("https://otlp.nr-data.net")
    );
    assert_eq!(
        built.config.exporters[0].owner,
        Some(ExporterOwner::Newrelic)
    );
    assert!(is_gated(&built.config.exporters[0]));
    assert!(!has_operator_exporter(&built.config));
    assert_eq!(outcome.config, built.config);
}

#[test]
fn langfuse_carrier_without_credentials_builds() {
    let fixture = fixture();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    add_carrier(&fixture, &mut in_memory, "langfuse_otel");
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "langfuse_otel", &mut in_memory)
        .expect("no failure")
        .expect("built");
    let records = fixture.constructor.records();
    let built = records.last().expect("the built logger");
    assert!(
        built
            .config
            .mapper_names
            .iter()
            .any(|name| name == "langfuse")
    );
    assert!(is_credential_gated(&built.config));
    assert!(exports_nowhere(&built.config));
    assert_eq!(outcome.config, built.config);
    assert_eq!(in_memory.len(), 2);
}

#[test]
fn a_destination_alone_does_not_carry_langfuse_credentials() {
    let fixture = fixture();
    fixture
        .trace
        .set_destinations(vec![destination("langfuse_otel")]);
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "langfuse_otel", &mut in_memory)
        .expect("no failure");
    assert!(outcome.is_none());
    assert!(in_memory.is_empty());
    assert_eq!(fixture.constructor.records().len(), 0);
}

#[test]
fn a_v2_logger_alone_does_not_carry_langfuse_credentials() {
    let fixture = fixture();
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let carrier = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("carrier"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("carrier");
    in_memory.push(Arc::clone(&carrier.callback));
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "langfuse_otel", &mut in_memory)
        .expect("no failure");
    assert!(outcome.is_none());
    assert_eq!(in_memory.len(), 1);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn the_factory_langfuse_handle_stamps_the_root_span() {
    let (log, runtime) = recorded_runtime(fixture());
    let mut config = settings(vec![]);
    config.mapper_names = vec!["langfuse".to_owned()];
    let logger = build_otel_v2_logger(
        &runtime,
        config,
        Some("langfuse_otel"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("built");
    let Some(CallbackHooks::Langfuse(callback)) = &logger.hooks else {
        panic!("the factory kept the Langfuse handle");
    };
    callback
        .log_pre_api_call(&PreCallEvent {
            model: "gpt-4o",
            messages: &Json::Null,
            details: &pre_call_details(),
        })
        .expect("pre call");
    assert_root_log(
        &log,
        &[
            "langfuse.trace.name=checkout",
            "user.id=user-1",
            "session.id=session-1",
            "langfuse.trace.tags=[\"env:test\",\"beta\"]",
        ],
    );
}

#[test]
fn the_factory_langfuse_content_handle_stamps_the_root_io() {
    let (log, runtime) = recorded_runtime(fixture());
    let mut config = settings(vec![]);
    config.mapper_names = vec!["langfuse".to_owned()];
    config.capture_message_content = CaptureMessageContent::SpanAndEvent;
    let logger = build_otel_v2_logger(
        &runtime,
        config,
        Some("langfuse_otel"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("built");
    let Some(CallbackHooks::LangfuseContent(callback)) = &logger.hooks else {
        panic!("the factory kept the LangfuseContent handle");
    };
    let data: MetadataMap =
        serde_json::from_str(r#"{"messages": [{"role": "user", "content": "hello"}]}"#)
            .expect("request data");
    let response: OtelModelValue = serde_json::from_str(
        r#"{"choices": [{"message": {"role": "assistant", "content": "done"}}]}"#,
    )
    .expect("response");
    callback.async_post_call_success_hook(
        &data,
        &MetadataMap::new(),
        &OutputResponse::ModelResponse(&response),
    );
    assert_root_log(
        &log,
        &[
            "langfuse.observation.output=[{\"content\":\"done\",\"role\":\"assistant\"}]",
            "langfuse.observation.input=[{\"content\":\"hello\",\"role\":\"user\"}]",
        ],
    );
}

#[test]
fn credentialed_preset_without_carrier_builds() {
    let fixture = fixture();
    fixture
        .environment
        .set("NEW_RELIC_LICENSE_KEY", "license-key");
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("built");
    let records = fixture.constructor.records();
    assert_eq!(records[0].config.exporters.len(), 2);
    assert!(!is_credential_gated(&records[0].config));
    assert_eq!(outcome.config, records[0].config);
}

#[test]
fn credentialed_preset_with_carrier_builds() {
    let fixture = fixture();
    fixture
        .environment
        .set("NEW_RELIC_LICENSE_KEY", "license-key");
    let mut in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    add_carrier(&fixture, &mut in_memory, "newrelic");
    let outcome = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("built");
    let records = fixture.constructor.records();
    let built = records.last().expect("the built logger");
    assert_eq!(built.config.exporters.len(), 2);
    assert!(!is_credential_gated(&built.config));
    assert_eq!(outcome.config, built.config);
}

#[test]
fn reuses_a_same_name_logger_that_exports() {
    let fixture = fixture();
    let existing = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![operator_exporter()]),
        Some("newrelic"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("existing");
    let mut in_memory = vec![Arc::clone(&existing.callback)];
    let reused = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("reused");
    assert!(Arc::ptr_eq(&reused.callback, &existing.callback));
    assert_eq!(in_memory.len(), 1);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn reuses_a_same_name_logger_that_serves_a_destination() {
    let fixture = fixture();
    let existing = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("newrelic"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("existing");
    fixture
        .trace
        .set_destinations(vec![destination("newrelic")]);
    let mut in_memory = vec![Arc::clone(&existing.callback)];
    let reused = maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory)
        .expect("no failure")
        .expect("reused");
    assert!(Arc::ptr_eq(&reused.callback, &existing.callback));
    assert_eq!(in_memory.len(), 1);
}

#[test]
fn skips_a_same_name_logger_that_exports_nowhere() {
    let fixture = fixture();
    let existing = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("newrelic"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("existing");
    let mut in_memory = vec![Arc::clone(&existing.callback)];
    let outcome =
        maybe_construct_otel_v2(&fixture.runtime, "newrelic", &mut in_memory).expect("no failure");
    assert!(outcome.is_none());
    assert_eq!(in_memory.len(), 1);
    assert_eq!(fixture.constructor.records().len(), 1);
}
