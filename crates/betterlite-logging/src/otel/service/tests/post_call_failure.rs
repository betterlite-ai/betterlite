use opentelemetry::trace::Status;

use super::fixtures::{error_config, exception, request_data};
use super::support::{Rig, START, attribute, event_attribute, i64_attribute, now, text_attribute};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_policy::error_recording::ERROR_STACK_TRACE;
use crate::otel::service::post_call::{
    FAILED_REQUEST_SPAN_NAME, PostCallFailure, ProxyCallKey, async_post_call_failure_hook,
};
use crate::otel::vocabulary::constants::attribute_keys::http;

const TRACEBACK: &str = "Traceback (most recent call last):\n  ValueError: boom\n";

fn facts(code: Option<&str>) -> ExceptionFacts {
    let raw = match code {
        Some(value) => format!("\"{value}\""),
        None => "null".to_owned(),
    };
    exception(&format!(
        r#"{{"class_name":"ValueError","message":"boom","code":{raw}}}"#
    ))
}

#[rstest::rstest]
#[case::without_error_code(None, 500, None)]
#[case::with_error_code(Some("402"), 402, None)]
#[case::with_traceback(Some("402"), 402, Some(TRACEBACK))]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] code: Option<&str>,
    #[case] status: i64,
    #[case] traceback: Option<&str>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let data = request_data("call-f", vec![]);
    let facts = facts(code);
    let key = ProxyCallKey {
        parent_span: Some(&parent),
        team_id: Some("team-a"),
        team_alias: Some("team-alias"),
    };
    let config = error_config();
    let call = PostCallFailure {
        request_data: &data,
        exception: Some(&facts),
        key: &key,
        traceback,
        info: &config,
    };
    async_post_call_failure_hook(&lifecycle, &call).expect("failure hook");
    let server = rig.span("proxy");
    assert_eq!(server.end_time, now());
    assert_eq!(server.status, Status::error(""));
    assert_eq!(i64_attribute(&server, http::RESPONSE_STATUS_CODE), status);
    assert_eq!(
        event_attribute(&server, "exception.type"),
        Some("ValueError".to_owned())
    );
    assert_eq!(
        event_attribute(&server, "exception.stacktrace"),
        Some("ValueError: boom\n".to_owned())
    );
    assert_eq!(
        event_attribute(&server, "exception.escaped"),
        Some("False".to_owned())
    );
    assert_eq!(
        text_attribute(&server, "metadata.user_api_key_team_id"),
        "team-a"
    );
    assert_eq!(text_attribute(&server, "error.type"), "ValueError");
    match code {
        Some(text) => assert_eq!(text_attribute(&server, "error.code"), text),
        None => assert!(attribute(&server, "error.code").is_none()),
    }
    match traceback {
        Some(text) => assert!(text_attribute(&server, ERROR_STACK_TRACE).starts_with(text)),
        None => assert!(attribute(&server, ERROR_STACK_TRACE).is_none()),
    }
    let child = rig.span(FAILED_REQUEST_SPAN_NAME);
    assert_eq!(child.parent_span_id, parent.context.span_id());
    assert_eq!(child.status, Status::error(""));
    assert_eq!(text_attribute(&child, "exception"), "boom");
    assert_eq!(
        text_attribute(&child, "metadata.user_api_key_team_alias"),
        "team-alias"
    );
    assert_eq!(child.end_time, now());
}
