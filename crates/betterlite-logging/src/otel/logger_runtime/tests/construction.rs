#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use chrono::FixedOffset;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::callback_logger::CallbackLogger;
use crate::otel::config::Settings;
use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::langfuse_root::{RootSpanSink, RootSpanSource};
use crate::otel::logger_runtime::construction::CallbackConstructor;
use crate::otel::logger_runtime::error::LoggerRuntimeError;
use crate::otel::logger_runtime::types::{
    BuiltLogger, ConstructorSettings, InjectedProviders, LoggerBuildRequest, LoggerClass,
    V2LoggerConstructor,
};
use crate::otel::v2_callback::error::CallbackError;

use super::support::{
    Fixture, NoSettings, finished, fixture, fixture_sink, probe, provider_exporting, settings,
};

struct NoRoots;

impl RootSpanSource for NoRoots {
    fn request_root_span(&self) -> Option<Arc<dyn RootSpanSink>> {
        None
    }
}

fn constructor() -> CallbackConstructor {
    CallbackConstructor {
        raw_config: otel_settings::Settings::default(),
        callback_settings: Arc::new(NoSettings),
        allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        roots: Arc::new(NoRoots),
        attribute_limit: None,
        redactor: None,
        log_client_error_tracebacks: false,
        utc_offset: FixedOffset::east_opt(0).expect("utc"),
    }
}

fn build(
    constructor: &CallbackConstructor,
    fixture: &Fixture,
    class: LoggerClass,
    config: &Settings,
    provider: &SdkTracerProvider,
    assembler: Option<&Arc<dyn StreamOutputAssembler>>,
    settings: ConstructorSettings,
) -> Result<BuiltLogger, LoggerRuntimeError> {
    constructor.build(&LoggerBuildRequest {
        class,
        config,
        callback_name: Some("generic"),
        providers: InjectedProviders {
            tracer_provider: Some(provider),
            logger_provider: None,
            meter_provider: None,
        },
        assembler,
        settings,
        factories: Arc::clone(&fixture.runtime.factories),
        scope_source: Arc::clone(&fixture.runtime.scope),
        trace: Arc::clone(&fixture.runtime.trace),
        environment: Arc::clone(&fixture.runtime.environment),
    })
}

#[test]
fn the_generic_class_builds_b3853_callback_over_the_injected_provider() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let built = build(
        &constructor(),
        &fixture,
        LoggerClass::Generic,
        &settings(vec![fixture_sink()]),
        &provider,
        None,
        ConstructorSettings::default(),
    )
    .expect("the generic logger builds");
    assert_eq!(built.callback.callback_name(), "generic");
    probe(&built.provider);
    assert_eq!(finished(&fixture.sdk_sink), 1);
}

#[test]
fn the_langfuse_class_builds_without_the_stream_assembler() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let built = build(
        &constructor(),
        &fixture,
        LoggerClass::Langfuse,
        &settings(vec![fixture_sink()]),
        &provider,
        None,
        ConstructorSettings::default(),
    )
    .expect("the Langfuse logger builds");
    assert_eq!(built.callback.callback_name(), "generic");
}

#[test]
fn the_langfuse_content_class_needs_the_runtime_assembler() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let config = settings(vec![fixture_sink()]);
    let result = build(
        &constructor(),
        &fixture,
        LoggerClass::LangfuseContent,
        &config,
        &provider,
        None,
        ConstructorSettings::default(),
    );
    assert!(matches!(result, Err(LoggerRuntimeError::MissingAssembler)));
    let built = build(
        &constructor(),
        &fixture,
        LoggerClass::LangfuseContent,
        &config,
        &provider,
        Some(&fixture.runtime.assembler),
        ConstructorSettings::default(),
    )
    .expect("the content logger builds");
    assert_eq!(built.callback.callback_name(), "generic");
}

#[test]
fn an_unknown_mapper_name_fails_the_construction() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let mut config = settings(vec![fixture_sink()]);
    config.mapper_names = vec!["nope".to_owned()];
    let result = build(
        &constructor(),
        &fixture,
        LoggerClass::Generic,
        &config,
        &provider,
        None,
        ConstructorSettings::default(),
    );
    assert!(matches!(
        result,
        Err(LoggerRuntimeError::Constructor {
            source: CallbackError::Mapper(_)
        })
    ));
}

#[test]
fn an_event_recorder_keeps_the_provider_it_reads() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let mut config = settings(vec![fixture_sink()]);
    config.enable_events = true;
    let built = build(
        &constructor(),
        &fixture,
        LoggerClass::Generic,
        &config,
        &provider,
        None,
        ConstructorSettings::default(),
    )
    .expect("the generic logger builds");
    assert!(built.recorder_providers.1.is_some());
}

#[test]
fn every_production_callback_reports_the_constructor_logging_settings() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let config = settings(vec![fixture_sink()]);
    let quiet = ConstructorSettings {
        turn_off_message_logging: true,
        message_logging: false,
    };
    let generic = build(
        &constructor(),
        &fixture,
        LoggerClass::Generic,
        &config,
        &provider,
        None,
        quiet,
    )
    .expect("the generic logger builds");
    assert!(generic.callback.turn_off_message_logging());
    assert!(!generic.callback.message_logging());
    assert!(!generic.callback.redacts_messages_itself());
    let langfuse = build(
        &constructor(),
        &fixture,
        LoggerClass::Langfuse,
        &config,
        &provider,
        None,
        quiet,
    )
    .expect("the Langfuse logger builds");
    assert!(langfuse.callback.turn_off_message_logging());
    assert!(!langfuse.callback.message_logging());
    assert!(!langfuse.callback.redacts_messages_itself());
    let content = build(
        &constructor(),
        &fixture,
        LoggerClass::LangfuseContent,
        &config,
        &provider,
        Some(&fixture.runtime.assembler),
        quiet,
    )
    .expect("the content logger builds");
    assert!(content.callback.turn_off_message_logging());
    assert!(!content.callback.message_logging());
    assert!(!content.callback.redacts_messages_itself());
}

#[test]
fn every_production_callback_keeps_the_default_logging_controls() {
    let fixture = fixture();
    let provider = provider_exporting(&fixture.sdk_sink);
    let config = settings(vec![fixture_sink()]);
    let defaults = ConstructorSettings::default();
    let generic = build(
        &constructor(),
        &fixture,
        LoggerClass::Generic,
        &config,
        &provider,
        None,
        defaults,
    )
    .expect("the generic logger builds");
    assert!(!generic.callback.turn_off_message_logging());
    assert!(generic.callback.message_logging());
    assert!(!generic.callback.redacts_messages_itself());
    let langfuse = build(
        &constructor(),
        &fixture,
        LoggerClass::Langfuse,
        &config,
        &provider,
        None,
        defaults,
    )
    .expect("the Langfuse logger builds");
    assert!(!langfuse.callback.turn_off_message_logging());
    assert!(langfuse.callback.message_logging());
    assert!(!langfuse.callback.redacts_messages_itself());
    let content = build(
        &constructor(),
        &fixture,
        LoggerClass::LangfuseContent,
        &config,
        &provider,
        Some(&fixture.runtime.assembler),
        defaults,
    )
    .expect("the content logger builds");
    assert!(!content.callback.turn_off_message_logging());
    assert!(content.callback.message_logging());
    assert!(!content.callback.redacts_messages_itself());
}
