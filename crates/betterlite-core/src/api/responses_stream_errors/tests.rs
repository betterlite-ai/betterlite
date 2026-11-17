#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde_json::Number;
use uuid::Uuid;

use super::error_codes::{is_decimal, response_error_code, status_error_code};
use super::failure::{FailureCause, FailureCode, FailureDetails, FailureKind};
use super::{ResponsesStreamErrorState, epoch_seconds};

const FIXED_TIME: u64 = 1_800_000_000;
const FIXED_UUID: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;

fn time_at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("a representable time")
}

fn state(redactor: Option<Arc<Redactor>>) -> ResponsesStreamErrorState {
    ResponsesStreamErrorState::new(
        Arc::new(|| time_at(FIXED_TIME)),
        Arc::new(|| Uuid::from_u128(FIXED_UUID)),
        redactor,
    )
}

fn state_without_sources() -> ResponsesStreamErrorState {
    let now: Arc<dyn Fn() -> SystemTime + Send + Sync> =
        Arc::new(|| -> SystemTime { panic!("the committed identity must not reach the clock") });
    let next_id: Arc<dyn Fn() -> Uuid + Send + Sync> =
        Arc::new(|| -> Uuid { panic!("the committed identity must not reach the uuid source") });
    ResponsesStreamErrorState::new(now, next_id, None)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn object(pairs: Vec<(&str, Json)>) -> Json {
    let mut fields = indexmap::IndexMap::new();
    for (key, value) in pairs {
        fields.insert(key.to_owned(), value);
    }
    Json::Object(fields)
}

fn plain(message: &str) -> FailureCause {
    FailureCause {
        kind: FailureKind::Plain,
        text: message.to_owned(),
        message: Some(text(message)),
        error_type: None,
        code: None,
        status_code: None,
        body: None,
        original_exception: None,
    }
}

fn fallback(original: FailureCause) -> FailureCause {
    FailureCause {
        kind: FailureKind::MidStreamFallback,
        text: "wrapper".to_owned(),
        message: Some(text("wrapper")),
        error_type: None,
        code: None,
        status_code: Some(number(599)),
        body: None,
        original_exception: Some(Box::new(original)),
    }
}

fn failed_frame(emitted: Option<String>) -> String {
    emitted.expect("the failure emits a frame")
}

fn payload(frame: &str) -> Json {
    let data = frame
        .strip_prefix("event: response.failed\ndata: ")
        .expect("the frame carries the response.failed event line")
        .strip_suffix("\n\n")
        .expect("the frame ends with a blank line");
    serde_json::from_str(data).expect("the data line is json")
}

fn member<'value>(value: &'value Json, key: &str) -> &'value Json {
    const NULL: Json = Json::Null;
    match value {
        Json::Object(fields) => fields.get(key).unwrap_or(&NULL),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &NULL,
    }
}

fn json_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        other => panic!("expected text, found {other:?}"),
    }
}

fn error_of(frame: &str) -> Json {
    member(member(&payload(frame), "response"), "error").clone()
}

fn error_code(frame: &str) -> String {
    json_text(member(&error_of(frame), "code"))
}

fn error_message(frame: &str) -> String {
    json_text(member(&error_of(frame), "message"))
}

#[test]
fn first_failure_emits_one_response_failed_frame() {
    let mut state = state(None);
    let frame = failed_frame(state.format_failure(&plain("broken")));
    assert_eq!(
        frame,
        concat!(
            "event: response.failed\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_0123456789abcdef0123456789abcdef\",",
            "\"created_at\":1800000000,\"error\":{\"code\":\"server_error\",\"message\":\"broken\"},",
            "\"object\":\"response\",\"output\":[],\"status\":\"failed\"},\"sequence_number\":0}\n\n"
        )
    );
    assert_eq!(state.format_failure(&plain("again")), None);
    assert!(state.terminal_emitted);
}

#[test]
fn a_cause_without_detail_attributes_still_emits_a_frame() {
    let mut failure = plain("broken");
    failure.message = None;
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "broken");
    assert_eq!(error_code(&frame), "server_error");
}

#[test]
fn empty_exception_message_falls_back_to_the_exception_text() {
    let mut failure = plain("from the exception");
    failure.message = Some(text(""));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "from the exception");
}

#[test]
fn committed_identity_is_used_without_generating_new_values() {
    let mut state = state_without_sources();
    state
        .observe_chunk(&object(vec![
            ("type", text("response.created")),
            ("sequence_number", number(4)),
            (
                "response",
                object(vec![
                    ("id", text("resp_a")),
                    ("model", text("m")),
                    ("created_at", number(0)),
                ]),
            ),
        ]))
        .expect("an event mapping");
    assert_eq!(state.response_id, None);
    assert_eq!(state.sequence_number, -1);
    assert_eq!(
        state.mark_emitted("frame-created".to_owned()),
        "frame-created"
    );
    assert_eq!(state.response_id.as_deref(), Some("resp_a"));
    assert_eq!(state.model.as_deref(), Some("m"));
    assert_eq!(state.created_at, Some(0));
    assert_eq!(state.sequence_number, 4);
    state
        .observe_chunk(&object(vec![
            ("type", text("response.output_text.delta")),
            ("sequence_number", number(2)),
        ]))
        .expect("an event mapping");
    assert_eq!(state.mark_emitted("frame-delta".to_owned()), "frame-delta");
    assert_eq!(state.sequence_number, 4);
    let frame = failed_frame(state.format_failure(&plain("broken")));
    assert_eq!(
        frame,
        concat!(
            "event: response.failed\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_a\",\"created_at\":0,",
            "\"error\":{\"code\":\"server_error\",\"message\":\"broken\"},\"model\":\"m\",",
            "\"object\":\"response\",\"output\":[],\"status\":\"failed\"},\"sequence_number\":5}\n\n"
        )
    );
}

#[test]
fn mark_emitted_returns_the_bytes_it_was_given() {
    let mut state = state_without_sources();
    state
        .observe_chunk(&object(vec![("sequence_number", number(3))]))
        .expect("an event mapping");
    let frame = b"event: response.output_text.delta\ndata: {}\n\n".to_vec();
    assert_eq!(state.mark_emitted(frame.clone()), frame);
    assert_eq!(state.sequence_number, 3);
}

#[test]
fn observed_identity_is_not_used_before_mark_emitted() {
    let mut state = state(None);
    state
        .observe_chunk(&object(vec![
            ("type", text("response.created")),
            ("sequence_number", number(4)),
            (
                "response",
                object(vec![
                    ("id", text("resp_a")),
                    ("model", text("m")),
                    ("created_at", number(0)),
                ]),
            ),
        ]))
        .expect("an event mapping");
    let payload = payload(&failed_frame(state.format_failure(&plain("broken"))));
    let response = member(&payload, "response");
    assert_eq!(
        json_text(member(response, "id")),
        "resp_0123456789abcdef0123456789abcdef"
    );
    assert_eq!(member(response, "created_at"), &number(1_800_000_000));
    assert_eq!(member(response, "model"), &Json::Null);
    assert_eq!(member(&payload, "sequence_number"), &number(0));
}

#[test]
fn empty_observed_identity_does_not_erase_committed_values() {
    let mut state = state_without_sources();
    state.response_id = Some("resp_a".to_owned());
    state.model = Some("m".to_owned());
    state.created_at = Some(0);
    state.sequence_number = 4;
    state
        .observe_chunk(&object(vec![
            ("type", text("response.in_progress")),
            (
                "response",
                object(vec![
                    ("id", text("")),
                    ("model", text("")),
                    ("created_at", Json::Null),
                ]),
            ),
        ]))
        .expect("an event mapping");
    assert_eq!(state.mark_emitted("frame".to_owned()), "frame");
    assert_eq!(state.response_id.as_deref(), Some("resp_a"));
    assert_eq!(state.model.as_deref(), Some("m"));
    assert_eq!(state.created_at, Some(0));
    assert_eq!(state.sequence_number, 4);
    let frame = failed_frame(state.format_failure(&plain("broken")));
    assert_eq!(
        json_text(member(member(&payload(&frame), "response"), "id")),
        "resp_a"
    );
}

#[test]
fn an_empty_committed_id_is_replaced_by_a_generated_one() {
    let mut state = state(None);
    state.response_id = Some(String::new());
    let frame = failed_frame(state.format_failure(&plain("broken")));
    assert_eq!(
        json_text(member(member(&payload(&frame), "response"), "id")),
        "resp_0123456789abcdef0123456789abcdef"
    );
}

#[test]
fn non_mapping_chunk_clears_the_pending_event() {
    let mut state = state_without_sources();
    state
        .observe_chunk(&object(vec![
            ("sequence_number", number(9)),
            (
                "response",
                object(vec![
                    ("id", text("resp_b")),
                    ("model", text("m2")),
                    ("created_at", number(7)),
                ]),
            ),
        ]))
        .expect("an event mapping");
    state
        .observe_chunk(&Json::List(vec![number(1)]))
        .expect("a non-mapping chunk is ignored");
    assert_eq!(state.mark_emitted("frame".to_owned()), "frame");
    assert_eq!(state.response_id, None);
    assert_eq!(state.model, None);
    assert_eq!(state.created_at, None);
    assert_eq!(state.sequence_number, -1);
    assert!(!state.terminal_emitted);
}

#[rstest]
#[case::completed("response.completed")]
#[case::failed("response.failed")]
#[case::incomplete("response.incomplete")]
fn observed_terminal_event_blocks_failure_only_once_emitted(#[case] event_type: &str) {
    let mut observed = state(None);
    observed
        .observe_chunk(&object(vec![("type", text(event_type))]))
        .expect("an event mapping");
    assert!(observed.format_failure(&plain("broken")).is_some());
    assert_eq!(observed.format_failure(&plain("broken")), None);

    let mut emitted = state(None);
    emitted
        .observe_chunk(&object(vec![("type", text(event_type))]))
        .expect("an event mapping");
    assert_eq!(emitted.mark_emitted("frame".to_owned()), "frame");
    assert_eq!(emitted.format_failure(&plain("broken")), None);
}

#[test]
fn body_mapping_overrides_exception_details() {
    let mut failure = plain("mapped");
    failure.code = Some(text("old"));
    failure.error_type = Some(text("old"));
    failure.status_code = Some(number(400));
    failure.body = Some(object(vec![
        ("message", text("upstream")),
        ("code", number(0)),
        ("type", text("provider")),
        ("status_code", number(500)),
    ]));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "upstream");
    assert_eq!(error_code(&frame), "invalid_request_error");
}

#[rstest]
#[case::overrides(text("provider"), "invalid_request_error")]
#[case::empty_falls_back(text(""), "insufficient_quota")]
fn body_type_overrides_or_falls_back(#[case] body_type: Json, #[case] expected: &str) {
    let mut failure = plain("mapped");
    failure.error_type = Some(text("insufficient_quota"));
    failure.status_code = Some(number(400));
    failure.body = Some(object(vec![("type", body_type)]));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_code(&frame), expected);
}

#[test]
fn empty_body_text_falls_back_but_a_zero_code_does_not() {
    let mut failure = plain("mapped");
    failure.code = Some(text("old"));
    failure.error_type = Some(text("old"));
    failure.status_code = Some(number(429));
    failure.body = Some(object(vec![
        ("message", text("")),
        ("type", text("")),
        ("code", number(0)),
    ]));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "mapped");
    assert_eq!(error_code(&frame), "rate_limit_exceeded");
}

#[test]
fn non_text_body_values_become_null_and_fall_back() {
    let mut failure = plain("mapped");
    failure.code = Some(text("old"));
    failure.error_type = Some(text("old"));
    failure.body = Some(object(vec![
        ("message", number(5)),
        ("type", Json::Bool(true)),
        ("code", Json::Bool(false)),
    ]));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "mapped");
    assert_eq!(error_code(&frame), "old");
}

#[test]
fn non_text_exception_attributes_become_null() {
    let mut failure = plain("text message");
    failure.message = Some(number(5));
    failure.error_type = Some(Json::Bool(true));
    failure.code = Some(Json::Bool(true));
    failure.status_code = Some(number(404));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "text message");
    assert_eq!(error_code(&frame), "not_found_error");
}

#[test]
fn non_mapping_body_leaves_attributes_alone() {
    let mut failure = plain("mapped");
    failure.code = Some(text("old"));
    failure.body = Some(Json::List(vec![number(1)]));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "mapped");
    assert_eq!(error_code(&frame), "old");
}

#[test]
fn body_status_code_is_ignored_for_the_error_code() {
    let mut failure = plain("mapped");
    failure.status_code = Some(number(403));
    failure.body = Some(object(vec![("status_code", number(500))]));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_code(&frame), "permission_error");
}

#[test]
fn mid_stream_fallback_unwraps_to_the_original_exception() {
    let mut original = plain("inner");
    original.status_code = Some(number(408));
    let failure = fallback(fallback(original));
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "inner");
    assert_eq!(error_code(&frame), "request_timeout");
}

#[test]
fn mid_stream_fallback_without_an_original_keeps_its_own_details() {
    let mut failure = fallback(plain("inner"));
    failure.original_exception = None;
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(error_message(&frame), "wrapper");
    assert_eq!(error_code(&frame), "server_error");
}

#[test]
fn missing_message_uses_the_exception_text_before_redaction() {
    let mut failure = plain("upstream http://service.internal:8080/v1 failed");
    failure.message = None;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor");
    let frame = failed_frame(state(Some(Arc::new(redactor))).format_failure(&failure));
    assert_eq!(
        error_message(&frame),
        "upstream http://REDACTED:8080/v1 failed"
    );
}

#[test]
fn enabled_redaction_rewrites_internal_details() {
    let mut failure = plain("upstream http://service.internal:8080/v1 failed");
    failure.text = "ignored".to_owned();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor");
    let frame = failed_frame(state(Some(Arc::new(redactor))).format_failure(&failure));
    assert_eq!(
        error_message(&frame),
        "upstream http://REDACTED:8080/v1 failed"
    );
}

#[test]
fn disabled_redaction_leaves_the_message_alone() {
    let mut failure = plain("upstream http://service.internal:8080/v1 failed");
    failure.text = "ignored".to_owned();
    let frame = failed_frame(state(None).format_failure(&failure));
    assert_eq!(
        error_message(&frame),
        "upstream http://service.internal:8080/v1 failed"
    );
}

#[rstest]
#[case::type_not_text(object(vec![("type", number(5))]))]
#[case::sequence_number_not_integer(object(vec![("sequence_number", text("4"))]))]
#[case::response_not_mapping(object(vec![("response", number(1))]))]
#[case::response_id_not_text(object(vec![("response", object(vec![("id", number(1))]))]))]
#[case::created_at_not_integer(object(vec![("response", object(vec![("created_at", text("now"))]))]))]
fn observe_chunk_rejects_wrongly_typed_members(#[case] chunk: Json) {
    let mut state = state(None);
    assert!(state.observe_chunk(&chunk).is_err());
    assert_eq!(state.response_id, None);
}

#[test]
fn a_rejected_chunk_leaves_the_staged_event_in_place() {
    let mut state = state_without_sources();
    state
        .observe_chunk(&object(vec![
            ("sequence_number", number(3)),
            ("response", object(vec![("id", text("resp_a"))])),
        ]))
        .expect("an event mapping");
    assert!(
        state
            .observe_chunk(&object(vec![("type", number(5))]))
            .is_err()
    );
    assert_eq!(state.mark_emitted("frame".to_owned()), "frame");
    assert_eq!(state.response_id.as_deref(), Some("resp_a"));
    assert_eq!(state.sequence_number, 3);
}

#[rstest]
#[case::unauthenticated(Some(401), "authentication_error")]
#[case::forbidden(Some(403), "permission_error")]
#[case::missing(Some(404), "not_found_error")]
#[case::timeout(Some(408), "request_timeout")]
#[case::rate_limited(Some(429), "rate_limit_exceeded")]
#[case::bad_request(Some(400), "invalid_request_error")]
#[case::unprocessable(Some(422), "invalid_request_error")]
#[case::client_closed(Some(499), "invalid_request_error")]
#[case::absent(None, "server_error")]
#[case::below_bad_request(Some(399), "server_error")]
#[case::server_fault(Some(500), "server_error")]
fn status_codes_map_to_error_codes(#[case] status_code: Option<i64>, #[case] expected: &str) {
    assert_eq!(status_error_code(status_code), expected);
    let details = FailureDetails {
        message: None,
        code: None,
        error_type: None,
        status_code,
    };
    assert_eq!(response_error_code(&details), expected);
}

fn code_text(value: &str) -> Option<FailureCode> {
    Some(FailureCode::Text(value.to_owned()))
}

#[rstest]
#[case::quota_code(code_text("insufficient_quota"), None, None, "insufficient_quota")]
#[case::quota_type(None, Some("insufficient_quota"), None, "insufficient_quota")]
#[case::integer_429(
    Some(FailureCode::Integer(429)),
    None,
    Some(400),
    "rate_limit_exceeded"
)]
#[case::text_429(code_text("429"), None, Some(400), "rate_limit_exceeded")]
#[case::rate_limit_prefix(None, Some("rate_limit_custom"), Some(400), "rate_limit_exceeded")]
#[case::code_before_type(
    code_text("rate_limit_custom"),
    Some("insufficient_quota"),
    None,
    "rate_limit_exceeded"
)]
#[case::type_checked_after_code(
    code_text("custom"),
    Some("insufficient_quota"),
    None,
    "insufficient_quota"
)]
#[case::custom_code_wins_over_status(code_text("custom"), None, Some(400), "custom")]
#[case::decimal_code_falls_back(code_text("12"), None, Some(429), "rate_limit_exceeded")]
#[case::empty_code_falls_back(code_text(""), None, Some(404), "not_found_error")]
#[case::spaces_are_not_decimal(code_text("  "), None, Some(404), "  ")]
#[case::superscript_is_not_decimal(code_text("\u{b2}"), None, Some(404), "\u{b2}")]
#[case::arabic_digits_are_decimal(code_text("\u{660}"), None, Some(400), "invalid_request_error")]
#[case::type_429(None, Some("429"), Some(400), "rate_limit_exceeded")]
fn response_codes_follow_code_then_type_then_status(
    #[case] code: Option<FailureCode>,
    #[case] error_type: Option<&str>,
    #[case] status_code: Option<i64>,
    #[case] expected: &str,
) {
    let details = FailureDetails {
        message: None,
        code,
        error_type: error_type.map(str::to_owned),
        status_code,
    };
    assert_eq!(response_error_code(&details), expected);
}

#[rstest]
#[case::digits("12", true)]
#[case::arabic_digits("\u{660}12", true)]
#[case::empty("", false)]
#[case::letters("1a", false)]
#[case::space("  ", false)]
#[case::superscript("\u{b2}", false)]
fn is_decimal_matches_python(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(is_decimal(text), expected);
}

#[rstest]
#[case::at_epoch(0, 0)]
#[case::whole_second(1_800_000_000, 1_800_000_000)]
#[case::truncates_millis(1_800_000_001, 1_800_000_001)]
fn epoch_seconds_truncates_to_whole_seconds(#[case] offset: u64, #[case] expected: i64) {
    let now = time_at(offset)
        .checked_add(Duration::from_millis(900))
        .expect("a time");
    assert_eq!(epoch_seconds(now), expected);
}

#[rstest]
#[case::half_second_before(500, 0)]
#[case::whole_second_before(1_000, -1)]
#[case::second_and_half_before(1_500, -1)]
fn epoch_seconds_truncates_before_the_epoch(#[case] millis: u64, #[case] expected: i64) {
    let now = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_millis(millis))
        .expect("a time before the epoch");
    assert_eq!(epoch_seconds(now), expected);
}

#[test]
fn the_initial_state_has_no_identity_and_no_sequence() {
    let state = state(None);
    assert_eq!(state.response_id, None);
    assert_eq!(state.model, None);
    assert_eq!(state.created_at, None);
    assert_eq!(state.sequence_number, -1);
    assert!(!state.terminal_emitted);
}
