use super::fixtures::{database_env, payload, service_call};
use super::support::{Rig, START, attribute, text_attribute};
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::async_service_success_hook;

#[rstest::rstest]
#[case::call_type(None, "call_type", Some("query"))]
#[case::service(None, "service", Some("postgres"))]
#[case::caller(
    Some("completion:inner"),
    "litellm.service.caller",
    Some("completion:inner")
)]
#[case::missing_caller(None, "litellm.service.caller", None)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] caller: Option<&str>,
    #[case] key: &str,
    #[case] expected: Option<&str>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = payload(ServiceType::Postgres, "query", caller, None);
    let call = service_call(&payload, Some(&parent), None);
    async_service_success_hook(&lifecycle, &call, &database_env());
    let span = rig.span("postgres");
    match expected {
        Some(text) => assert_eq!(text_attribute(&span, key), text),
        None => assert!(attribute(&span, key).is_none()),
    }
}
