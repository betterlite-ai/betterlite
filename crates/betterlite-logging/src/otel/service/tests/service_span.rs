use opentelemetry::trace::{SpanKind, Status};
use opentelemetry_sdk::trace::SpanData;

use super::fixtures::{database_env, payload, service_call};
use super::support::{END, Rig, START, attribute, instant, text_attribute};
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::async_service_success_hook;
use crate::otel::vocabulary::constants::attribute_keys::db;

fn success_span(service: ServiceType, name: &str) -> SpanData {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = payload(service, "query", None, None);
    let call = service_call(&payload, Some(&parent), None);
    async_service_success_hook(&lifecycle, &call, &database_env());
    let span = rig.span(name);
    assert_eq!(
        span.parent_span_id,
        parent.context.span_id(),
        "the service span hangs off the request"
    );
    span
}

#[rstest::rstest]
#[case::postgres_is_a_client_span(
    ServiceType::Postgres,
    "postgres",
    SpanKind::Client,
    "postgresql"
)]
#[case::redis_is_a_client_span(ServiceType::Redis, "redis", SpanKind::Client, "redis")]
#[case::auth_is_an_internal_span(ServiceType::Auth, "auth", SpanKind::Internal, "")]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] service: ServiceType,
    #[case] name: &str,
    #[case] kind: SpanKind,
    #[case] system: &str,
) {
    let span = success_span(service, name);
    assert_eq!(span.name, name);
    assert_eq!(span.span_kind, kind);
    assert_eq!(span.start_time, instant(START));
    assert_eq!(span.end_time, instant(END));
    assert_eq!(span.status, Status::Ok);
    if system.is_empty() {
        assert!(attribute(&span, db::SYSTEM_NAME).is_none());
    } else {
        assert_eq!(text_attribute(&span, db::SYSTEM_NAME), system);
    }
}
