#![cfg(test)]

use opentelemetry::trace::{Span as _, Tracer as _};
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::{BASIC_AUTH, PUBLIC_KEY, Rig, SECRET_KEY, langfuse_headers};
use crate::langfuse_otel::error::LangfuseOtelError;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};

fn in_memory_config() -> OpenTelemetryConfig {
    let env = super::environment(&[("OTEL_EXPORTER", "in_memory")]);
    let mut config = OpenTelemetryConfig::from_env(&env);
    config.service_name = "mine".to_owned();
    config
}

fn exported_names(exporter: &InMemorySpanExporter) -> Vec<String> {
    exporter
        .get_finished_spans()
        .expect("finished spans")
        .iter()
        .map(|span| span.name.to_string())
        .collect()
}

#[test]
fn initialization_without_a_config_builds_it_from_the_langfuse_environment() {
    let rig = Rig::new(&[
        ("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY),
        ("LANGFUSE_SECRET_KEY", SECRET_KEY),
        ("LANGFUSE_HOST", "lf.example.com"),
    ]);
    let logger = rig.logger(None).expect("logger");
    let config = &logger.otel.config;
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://lf.example.com/api/public/otel")
    );
    assert_eq!(config.headers, Some(langfuse_headers(BASIC_AUTH)));
    assert_eq!(logger.otel.callback_name.as_deref(), Some("langfuse_otel"));
}

#[test]
fn initialization_without_credentials_falls_back_to_the_otel_environment() {
    let rig = Rig::new(&[
        ("OTEL_EXPORTER", "console"),
        ("OTEL_SERVICE_NAME", "fallback"),
    ]);
    let logger = rig.logger(None).expect("logger");
    assert_eq!(logger.otel.config.exporter, Exporter::Console);
    assert_eq!(logger.otel.config.endpoint, None);
    assert_eq!(logger.otel.config.service_name, "fallback");
}

#[test]
fn initialization_keeps_a_supplied_config_and_reaches_the_parent_init() {
    let rig = Rig::new(&[
        ("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY),
        ("LANGFUSE_SECRET_KEY", SECRET_KEY),
    ]);
    let logger = rig.logger(Some(in_memory_config())).expect("logger");
    assert_eq!(logger.otel.config.exporter, Exporter::InMemory);
    assert_eq!(logger.otel.config.service_name, "mine");
    logger.otel.tracer.start("llm-call").end();
    assert_eq!(exported_names(&rig.exporter), vec!["llm-call".to_owned()]);
}

#[test]
fn initialization_reports_a_parent_init_failure_with_its_cause() {
    let rig = Rig::new(&[]);
    let mut config = in_memory_config();
    config.exporter = Exporter::Named("otlp_http".to_owned());
    config.endpoint = Some("https://lf.example.com/api/public/otel".to_owned());
    config.headers = Some("no-separator".to_owned());
    let Err(error) = rig.logger(Some(config)) else {
        panic!("the headers are invalid");
    };
    assert!(matches!(error, LangfuseOtelError::Init { .. }));
}
