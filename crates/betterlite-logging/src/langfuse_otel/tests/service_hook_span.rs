#![cfg(test)]

use std::collections::BTreeMap;

use chrono::Utc;
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::Rig;
use crate::langfuse_otel::service_hooks::ServiceHooks as _;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;

#[test]
fn service_hook_span_creates_no_proxy_request_span() {
    let rig = Rig::new(&[("OTEL_EXPORTER", "in_memory")]);
    let env = rig.env.clone();
    let mut config = OpenTelemetryConfig::from_env(&env);
    config.normalize(&env);
    let logger = rig.logger(Some(config)).expect("logger");
    let headers = BTreeMap::from([("traceparent".to_owned(), "00-1-2-01".to_owned())]);
    let span = logger.request_started_span(Utc::now(), &headers);
    assert!(span.is_none());
    let exporter: &InMemorySpanExporter = &rig.exporter;
    assert!(exporter.get_finished_spans().expect("spans").is_empty());
}
