use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::SpanData;

use super::fixtures::{error_config, exception};
use super::support::{
    Rig, START, attribute, event_attribute, i64_attribute, instant, kwargs, text_attribute,
};
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::service::management::{ManagementPayload, async_management_failure_hook};
use crate::otel::vocabulary::constants::attribute_keys::http;

fn endpoint_span(rig: &Rig, facts: bool) -> SpanData {
    let span = rig.span("POST /key/delete");
    assert_eq!(span.start_time, instant(1_704_067_200.0));
    assert_eq!(span.end_time, instant(1_704_067_202.0));
    assert_eq!(span.status, Status::error(""));
    assert_eq!(text_attribute(&span, "request.key"), "sk-1");
    assert_eq!(
        text_attribute(&span, "exception"),
        if facts { "boom" } else { "None" }
    );
    span
}

#[rstest::rstest]
#[case::exception_without_code(Some(r#"{"class_name":"ValueError","message":"boom"}"#), None)]
#[case::exception_with_numeric_code(
    Some(r#"{"class_name":"ValueError","message":"boom","code":"402"}"#),
    Some(402)
)]
#[case::no_exception(None, None)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] facts_json: Option<&str>,
    #[case] expected: Option<i64>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = ManagementPayload {
        route: "POST /key/delete".to_owned(),
        request_data: kwargs(r#"{"key":"sk-1"}"#),
        response: None,
        exception: facts_json.map(exception),
        start_time: SpanTime::Seconds(1_704_067_200.9),
        end_time: SpanTime::Seconds(1_704_067_202.7),
    };
    let config = error_config();
    async_management_failure_hook(&lifecycle, &payload, Some(&parent), &config);
    endpoint_span(&rig, facts_json.is_some());
    let server = rig.span("proxy");
    assert_eq!(server.status, Status::error(""));
    assert_eq!(server.end_time, instant(1_704_067_202.0));
    if facts_json.is_none() {
        assert!(server.events.is_empty(), "no exception, no event");
        assert!(attribute(&server, "error.type").is_none());
        return;
    }
    assert_eq!(
        event_attribute(&server, "exception.type"),
        Some("ValueError".to_owned())
    );
    assert_eq!(text_attribute(&server, "error.type"), "ValueError");
    assert_eq!(text_attribute(&server, "error.message"), "boom");
    if let Some(code) = expected {
        assert_eq!(text_attribute(&server, "error.code"), code.to_string());
        assert_eq!(i64_attribute(&server, http::RESPONSE_STATUS_CODE), code);
    } else {
        assert!(attribute(&server, "error.code").is_none());
        assert!(attribute(&server, http::RESPONSE_STATUS_CODE).is_none());
    }
}
