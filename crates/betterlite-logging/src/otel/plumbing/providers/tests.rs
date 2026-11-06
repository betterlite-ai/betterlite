#![cfg(test)]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use opentelemetry::logs::{LogRecord as _, Logger as _};
use opentelemetry_sdk::logs::InMemoryLogExporter;
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
use rstest::rstest;

use super::{
    METRIC_EXPORT_INTERVAL, ProviderState, Resolved, build_logger_provider, build_resource,
    get_meter, resolve_logger_provider, resolve_meter_provider, resolve_provider,
};
use crate::otel::plumbing::exporters::tests::{RecordingClient, clients, config};
use crate::otel::plumbing::exporters::{ExporterError, build_log_exporter};
use crate::otel::plumbing::providers::get_event_logger;
use crate::otel::plumbing::sinks::LogSink;

fn resource_pairs(config: &OtelConfig) -> BTreeMap<String, String> {
    build_resource(config)
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn exporter_metric_interval_is_five_seconds() {
    assert_eq!(METRIC_EXPORT_INTERVAL, Duration::from_secs(5));
}

#[rstest]
#[case::name_only(None, &[], &[("service.name", "litellm-proxy")])]
#[case::environment(
    Some("prod"),
    &[],
    &[("service.name", "litellm-proxy"), ("deployment.environment", "prod")]
)]
#[case::empty_environment_kept(
    Some(""),
    &[],
    &[("service.name", "litellm-proxy"), ("deployment.environment", "")]
)]
#[case::extra_attribute(
    None,
    &[("team", "blue")],
    &[("service.name", "litellm-proxy"), ("team", "blue")]
)]
#[case::extra_overrides_service_name(
    None,
    &[("service.name", "custom")],
    &[("service.name", "custom")]
)]
#[case::extra_overrides_environment(
    Some("prod"),
    &[("deployment.environment", "staging")],
    &[("service.name", "litellm-proxy"), ("deployment.environment", "staging")]
)]
fn exporter_resource_merges_settings_and_extra_attributes(
    #[case] environment: Option<&str>,
    #[case] extra: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let mut settings = config("console");
    settings.deployment_environment = environment.map(str::to_owned);
    settings.resource_attributes = extra
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let wanted: BTreeMap<String, String> = expected
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let built = resource_pairs(&settings);
    for (name, value) in &wanted {
        assert_eq!(built.get(name), Some(value), "{name}");
    }
    assert_eq!(
        built
            .keys()
            .filter(|name| name.starts_with("deployment."))
            .count(),
        wanted
            .keys()
            .filter(|name| name.starts_with("deployment."))
            .count()
    );
}

#[rstest]
#[case::injected_wins(Some(1), ProviderState::Configured(2), Resolved::Reuse(1), false)]
#[case::injected_over_opt_out(Some(1), ProviderState::OptedOut, Resolved::Reuse(1), false)]
#[case::existing_reused(None, ProviderState::Configured(2), Resolved::Reuse(2), false)]
#[case::opted_out(None, ProviderState::OptedOut, Resolved::Disabled, false)]
#[case::built_when_unset(None, ProviderState::Unset, Resolved::Publish(9), true)]
fn exporter_provider_resolution_prefers_injected_then_existing(
    #[case] injected: Option<u32>,
    #[case] existing: ProviderState<u32>,
    #[case] expected: Resolved<u32>,
    #[case] builds: bool,
) {
    let built = Cell::new(false);
    let resolved = resolve_provider(injected, existing, || {
        built.set(true);
        Ok(9)
    })
    .expect("resolved");
    let matches_expected = match (resolved, expected) {
        (Resolved::Reuse(left), Resolved::Reuse(right))
        | (Resolved::Publish(left), Resolved::Publish(right)) => left == right,
        (Resolved::Disabled, Resolved::Disabled) => true,
        _ => false,
    };
    assert!(matches_expected);
    assert_eq!(built.get(), builds);
}

#[test]
fn exporter_provider_resolution_returns_the_build_error() {
    let outcome: Result<Resolved<u32>, ExporterError> =
        resolve_provider(None, ProviderState::Unset, || {
            Err(ExporterError::UnknownKind("x".to_owned()))
        });
    assert!(matches!(outcome, Err(ExporterError::UnknownKind(_))));
}

#[test]
fn exporter_meter_provider_is_published_only_for_an_unset_process() {
    let recorder = Arc::new(RecordingClient::default());
    let wired = clients(&recorder);
    let settings = config("console");
    let unset = resolve_meter_provider(&settings, &wired, None, ProviderState::Unset);
    assert!(matches!(unset, Ok(Resolved::Publish(_))));
    let opted_out = resolve_meter_provider(&settings, &wired, None, ProviderState::OptedOut);
    assert!(matches!(opted_out, Ok(Resolved::Disabled)));
    let Ok(Resolved::Publish(provider)) = unset else {
        return;
    };
    let configured = resolve_meter_provider(
        &settings,
        &wired,
        None,
        ProviderState::Configured(provider.clone()),
    );
    assert!(matches!(configured, Ok(Resolved::Reuse(_))));
    let injected =
        resolve_meter_provider(&settings, &wired, Some(provider), ProviderState::OptedOut);
    assert!(matches!(injected, Ok(Resolved::Reuse(_))));
}

#[test]
fn exporter_logger_provider_resolution_follows_the_process_state() {
    let recorder = Arc::new(RecordingClient::default());
    let wired = clients(&recorder);
    let settings = config("in_memory");
    let unset = resolve_logger_provider(&settings, &wired, None, ProviderState::Unset);
    assert!(matches!(unset, Ok(Resolved::Publish(_))));
    let opted_out = resolve_logger_provider(&settings, &wired, None, ProviderState::OptedOut);
    assert!(matches!(opted_out, Ok(Resolved::Disabled)));
    let Ok(Resolved::Publish(provider)) = unset else {
        return;
    };
    let configured =
        resolve_logger_provider(&settings, &wired, None, ProviderState::Configured(provider));
    assert!(matches!(configured, Ok(Resolved::Reuse(_))));
}

#[test]
fn exporter_logger_provider_stamps_the_resource_on_records() {
    let exporter = InMemoryLogExporter::default();
    let provider = build_logger_provider(&config("in_memory"), LogSink::InMemory(exporter.clone()));
    let logger = get_event_logger(&provider, "litellm");
    logger.emit(logger.create_log_record());
    let logs = exporter.get_emitted_logs().expect("logs");
    let resource = &logs.first().expect("one log").resource;
    let service = resource
        .iter()
        .find(|(key, _)| key.as_str() == "service.name")
        .map(|(_, value)| value.to_string());
    assert_eq!(service.as_deref(), Some("litellm-proxy"));
}

#[test]
fn exporter_logger_provider_batches_network_sinks() {
    let recorder = Arc::new(RecordingClient::default());
    let mut settings = config("otlp_http");
    settings.endpoint = Some("http://collector:4318".to_owned());
    let sink = build_log_exporter(&settings, &clients(&recorder)).expect("sink");
    let provider = build_logger_provider(&settings, sink);
    let logger = get_event_logger(&provider, "litellm");
    logger.emit(logger.create_log_record());
    assert!(recorder.recorded().is_empty());
    provider.force_flush().expect("flush");
    let posted = recorder.recorded();
    assert_eq!(posted.len(), 1);
    assert_eq!(
        posted.first().map(|request| request.uri.as_str()),
        Some("http://collector:4318/v1/logs")
    );
}

#[test]
fn exporter_meter_scope_carries_the_name_and_crate_version() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    get_meter(&provider, "litellm.metrics")
        .u64_counter("probe")
        .build()
        .add(1, &[]);
    provider.force_flush().expect("flush");
    let metrics = exporter.get_finished_metrics().expect("metrics");
    let scope = metrics
        .first()
        .expect("exported metrics")
        .scope_metrics()
        .next()
        .expect("one scope")
        .scope();
    assert_eq!(scope.name(), "litellm.metrics");
    assert_eq!(scope.version(), Some(env!("CARGO_PKG_VERSION")));
}

#[test]
fn event_logger_scope_carries_the_name_and_crate_version() {
    let exporter = InMemoryLogExporter::default();
    let provider = build_logger_provider(&config("in_memory"), LogSink::InMemory(exporter.clone()));
    let logger = get_event_logger(&provider, "litellm.events");
    let mut record = logger.create_log_record();
    record.set_event_name("probe");
    logger.emit(record);
    let logs = exporter.get_emitted_logs().expect("logs");
    let scope = &logs.first().expect("one log").instrumentation;
    assert_eq!(scope.name(), "litellm.events");
    assert_eq!(scope.version(), Some(env!("CARGO_PKG_VERSION")));
}
