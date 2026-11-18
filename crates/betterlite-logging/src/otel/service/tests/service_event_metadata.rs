use opentelemetry::Value;

use super::fixtures::{database_env, payload, service_call};
use super::support::{Rig, START, attribute, kwargs};
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::async_service_success_hook;

#[rstest::rstest]
#[case::string(r#"{"cache.hit":"yes"}"#, "cache.hit", Value::from("yes"))]
#[case::null_becomes_none_text(r#"{"cache.hit":null}"#, "cache.hit", Value::from("None"))]
#[case::object_becomes_json(r#"{"cache.hit":{"a":1}}"#, "cache.hit", Value::from(r#"{"a":1}"#))]
#[case::number_stays_numeric(r#"{"cache.ttl":30}"#, "cache.ttl", Value::I64(30))]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] metadata: &str,
    #[case] key: &str,
    #[case] expected: Value,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = payload(ServiceType::Postgres, "query", None, None);
    let event_metadata = kwargs(metadata);
    let call = service_call(&payload, Some(&parent), Some(&event_metadata));
    async_service_success_hook(&lifecycle, &call, &database_env());
    let span = rig.span("postgres");
    assert_eq!(attribute(&span, key), Some(&expected));
}
