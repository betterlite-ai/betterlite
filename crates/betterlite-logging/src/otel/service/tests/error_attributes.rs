use opentelemetry::trace::Span as _;

use super::fixtures::{error_config, exception};
use super::support::{Rig, START, attribute, i64_attribute, text_attribute};
use crate::otel::service::error_attributes::record_error_attributes;
use crate::otel::vocabulary::constants::attribute_keys::http;

#[rstest::rstest]
#[case::exception_with_code(
    Some(r#"{"class_name":"ValueError","message":"boom","code":"402"}"#),
    true,
    Some("400")
)]
#[case::exception_without_code(
    Some(r#"{"class_name":"ValueError","message":"boom"}"#),
    true,
    Some("400")
)]
#[case::no_exception(None, true, None)]
#[case::no_span(
    Some(r#"{"class_name":"ValueError","message":"boom","code":"402"}"#),
    false,
    None
)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] facts_json: Option<&str>,
    #[case] has_span: bool,
    #[case] expected: Option<&str>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let facts = facts_json.map(exception);
    let config = error_config();
    record_error_attributes(has_span.then_some(&parent), facts.as_ref(), 400, &config);
    parent.lock().end();
    let span = rig.span("proxy");
    assert!(
        span.events.is_empty(),
        "the stamping emits no exception event"
    );
    match expected {
        Some(code) => {
            assert_eq!(text_attribute(&span, "error.code"), code);
            assert_eq!(text_attribute(&span, "error.type"), "ValueError");
            assert_eq!(text_attribute(&span, "error.message"), "boom");
            assert_eq!(i64_attribute(&span, http::RESPONSE_STATUS_CODE), 400);
        }
        None => assert!(attribute(&span, "error.code").is_none()),
    }
}
