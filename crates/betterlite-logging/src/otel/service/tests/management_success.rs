use opentelemetry::trace::Status;

use super::support::{Rig, START, attribute, i64_attribute, instant, kwargs, text_attribute};
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::service::management::{ManagementPayload, async_management_success_hook};
use crate::otel::vocabulary::constants::attribute_keys::http;

#[rstest::rstest]
#[case::with_response(true)]
#[case::without_response(false)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] with_response: bool) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let response = with_response.then_some(kwargs(r#"{"key":"sk-1"}"#));
    let payload = ManagementPayload {
        route: "POST /key/generate".to_owned(),
        request_data: kwargs(r#"{"model":"gpt-4o"}"#),
        response,
        exception: None,
        start_time: SpanTime::Seconds(1_704_067_201.25),
        end_time: SpanTime::Seconds(1_704_067_201.75),
    };
    async_management_success_hook(&lifecycle, &payload, Some(&parent));
    let span = rig.span("POST /key/generate");
    assert_eq!(span.parent_span_id, parent.context.span_id());
    assert_eq!(span.start_time, instant(1_704_067_201.25));
    assert_eq!(span.end_time, instant(1_704_067_201.75));
    assert_eq!(span.status, Status::Ok);
    assert_eq!(text_attribute(&span, "request.model"), "gpt-4o");
    assert!(
        span.events.is_empty(),
        "the endpoint span records no exception"
    );
    if with_response {
        assert_eq!(text_attribute(&span, "response.key"), "sk-1");
    } else {
        assert!(attribute(&span, "response.key").is_none());
    }
    let server = rig.span("proxy");
    assert_eq!(server.status, Status::Ok);
    assert_eq!(server.end_time, instant(1_704_067_201.75));
    assert_eq!(i64_attribute(&server, http::RESPONSE_STATUS_CODE), 200);
}
