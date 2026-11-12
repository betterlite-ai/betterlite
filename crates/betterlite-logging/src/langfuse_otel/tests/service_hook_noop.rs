#![cfg(test)]

use super::Rig;
use crate::langfuse_otel::service_hooks::ServiceHooks as _;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};

fn payload(error: Option<&str>) -> ServiceLoggerPayload {
    ServiceLoggerPayload {
        service: ServiceType::Postgres,
        call_type: "async_health_check".to_owned(),
        caller: Some("proxy".to_owned()),
        error: error.map(str::to_owned),
    }
}

#[tokio::test]
async fn service_hook_noop_success_and_failure_export_nothing() {
    let rig = Rig::new(&[("OTEL_EXPORTER", "in_memory")]);
    let env = rig.env.clone();
    let mut config = OpenTelemetryConfig::from_env(&env);
    config.normalize(&env);
    let logger = rig.logger(Some(config)).expect("logger");
    logger.async_service_success_hook(&payload(None)).await;
    logger
        .async_service_failure_hook(&payload(Some("connection refused")))
        .await;
    assert!(rig.exporter.get_finished_spans().expect("spans").is_empty());
}
