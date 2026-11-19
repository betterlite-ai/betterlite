use std::collections::BTreeMap;

use opentelemetry::trace::SpanKind;

use super::fixtures::{database_env, payload, service_call};
use super::support::{Rig, START, attribute, i64_attribute, text_attribute};
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::async_service_success_hook;
use crate::otel::vocabulary::constants::attribute_keys::{db, server};

#[rstest::rstest]
#[case::endpoint_from_database_url(true, Some("db.example.com"), Some(6543), Some("appdb"))]
#[case::no_endpoint_without_database_url(false, None, None, None)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] configured: bool,
    #[case] address: Option<&str>,
    #[case] port: Option<i64>,
    #[case] namespace: Option<&str>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = payload(ServiceType::Postgres, "insert", None, None);
    let call = service_call(&payload, Some(&parent), None);
    let env = if configured {
        database_env()
    } else {
        BTreeMap::new()
    };
    async_service_success_hook(&lifecycle, &call, &env);
    let span = rig.span("postgres");
    assert_eq!(span.span_kind, SpanKind::Client);
    assert_eq!(text_attribute(&span, db::OPERATION_NAME), "insert");
    match address {
        Some(text) => assert_eq!(text_attribute(&span, server::ADDRESS), text),
        None => assert!(attribute(&span, server::ADDRESS).is_none()),
    }
    match port {
        Some(number) => assert_eq!(i64_attribute(&span, server::PORT), number),
        None => assert!(attribute(&span, server::PORT).is_none()),
    }
    match namespace {
        Some(text) => assert_eq!(text_attribute(&span, db::NAMESPACE), text),
        None => assert!(attribute(&span, db::NAMESPACE).is_none()),
    }
}
