#![cfg(test)]

use std::sync::Arc;

use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::InMemorySpanExporter;
use rstest::rstest;

use crate::callback_logger::CallbackLogger;
use crate::otel::config::{CaptureMessageContent, ExporterSpec, Settings};
use crate::otel::logger_runtime::error::LoggerRuntimeError;
use crate::otel::logger_runtime::factory::{build_otel_v2_logger, logger_class};
use crate::otel::logger_runtime::preset::{
    exports_nowhere, has_operator_exporter, is_credential_gated, is_gated, maybe_construct_otel_v2,
    only_the_gated_exporter,
};
use crate::otel::logger_runtime::types::{ConstructorSettings, InjectedProviders, LoggerClass};
use crate::otel::mappers::registry::MapperError;
use crate::otel::v2_callback::error::CallbackError;

use super::support::{
    Fixture, destination, fixture, fixture_disabled, fixture_with_base, gated, operator_exporter,
    placeholder, provider_exporting, settings, spec,
};

fn build(fixture: &Fixture, config: Settings, name: &str) {
    build_otel_v2_logger(
        &fixture.runtime,
        config,
        Some(name),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .expect("built");
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

#[test]
fn factory_provider_passthrough() {
    let fixture = fixture();
    let sink = InMemorySpanExporter::default();
    let tracer = provider_exporting(&sink);
    let meter = SdkMeterProvider::default();
    let logs = SdkLoggerProvider::builder().build();
    let logger = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        None,
        InjectedProviders {
            tracer_provider: Some(&tracer),
            logger_provider: Some(&logs),
            meter_provider: Some(&meter),
        },
        ConstructorSettings::default(),
    )
    .expect("built");
    let records = fixture.constructor.records();
    assert_eq!(records[0].providers, vec!["tracer", "meter", "logger"]);
    assert_eq!(records[0].callback_name, None);
    assert_eq!(logger.callback.callback_name(), "generic");
}

#[test]
fn factory_chooses_generic_class() {
    let fixture = fixture();
    build(&fixture, settings(vec![]), "otel");
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
    build(&fixture, config, "langfuse_otel");
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
    build(&fixture, config, "langfuse_otel");
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
    build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("otel"),
        InjectedProviders::default(),
        constructor_settings,
    )
    .expect("generic");
    let mut langfuse = settings(vec![]);
    langfuse.mapper_names = vec!["langfuse".to_owned()];
    build_otel_v2_logger(
        &fixture.runtime,
        langfuse.clone(),
        Some("langfuse_otel"),
        InjectedProviders::default(),
        constructor_settings,
    )
    .expect("langfuse");
    langfuse.capture_message_content = CaptureMessageContent::SpanAndEvent;
    build_otel_v2_logger(
        &fixture.runtime,
        langfuse,
        Some("langfuse_otel"),
        InjectedProviders::default(),
        constructor_settings,
    )
    .expect("langfuse content");
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
#[case::absent_headers(None, true, true)]
#[case::empty_headers(Some(""), true, true)]
#[case::present_headers(Some("authorization=x"), true, false)]
#[case::header_optional(None, false, false)]
fn is_gated_table(
    #[case] headers: Option<&str>,
    #[case] requires_headers: bool,
    #[case] expected: bool,
) {
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
        error,
        LoggerRuntimeError::Constructor {
            source: CallbackError::Mapper(MapperError::Unknown { ref name, .. })
        } if name == "boom"
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
    assert_eq!(built.config.exporters, vec![gated()]);
    assert_eq!(outcome.config, built.config);
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
