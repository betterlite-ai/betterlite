use super::fixtures::{database_env, payload, service_call};
use super::support::{Rig, START, attribute, text_attribute};
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::async_service_failure_hook;
use crate::otel::vocabulary::constants::attribute_keys::error;

#[rstest::rstest]
#[case::text(Some("boom"), Some("boom"))]
#[case::empty_text(Some(""), None)]
#[case::absent(None, None)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] error_text: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = payload(ServiceType::Postgres, "query", None, None);
    let call = service_call(&payload, Some(&parent), None);
    async_service_failure_hook(&lifecycle, &call, error_text, &database_env());
    let span = rig.span("postgres");
    match expected {
        Some(text) => assert_eq!(text_attribute(&span, error::MESSAGE_LEGACY), text),
        None => assert!(attribute(&span, error::MESSAGE_LEGACY).is_none()),
    }
}
