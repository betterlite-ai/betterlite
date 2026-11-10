use rstest::rstest;

use super::support::{Knob, Look, look, run_post};

const RESPONSE: &str = r#"{"original_response":"raw text","input":"hello","additional_args":{"api_base":"https://host/v1?key=abcdef1234","headers":{"Authorization":"Bearer sk-live-abcdef123456"}}}"#;
const DICT_RESPONSE: &str = r#"{"original_response":{"id":"r1"}}"#;
const LOGGER_POST: &str = "logger_post:2026-01-01T00:00:00+00:00:";

#[rstest]
#[case::records_the_response(RESPONSE, &[], Look::Detail(&["original_response"]), r#""raw text""#)]
#[case::dict_response_is_stored_as_json_text(DICT_RESPONSE, &[], Look::Detail(&["original_response"]), r#""{\"id\": \"r1\"}""#)]
#[case::records_the_event_type(RESPONSE, &[], Look::Detail(&["log_event_type"]), r#""post_api_call""#)]
#[case::records_the_input(RESPONSE, &[], Look::Detail(&["input"]), r#""hello""#)]
#[case::missing_input_is_null(DICT_RESPONSE, &[], Look::Detail(&["input"]), "null")]
#[case::stored_header_is_masked(RESPONSE, &[], Look::Detail(&["additional_args", "headers", "Authorization"]), r#""Be****56""#)]
#[case::bearer_value_never_stored(RESPONSE, &[], Look::Leaks("sk-live-abcdef123456"), "clean")]
#[case::raw_response_logs_at_debug(RESPONSE, &[], Look::Record(0), "Debug:RAW RESPONSE:\nraw text\n\n:{}")]
#[case::raw_response_warns_for_request_debug(RESPONSE, &[Knob::RequestDebug], Look::Record(0), "Warning:RAW RESPONSE:\nraw text\n\n:{}")]
#[case::dict_response_logs_as_json(DICT_RESPONSE, &[], Look::Record(0), "Debug:RAW RESPONSE:\n{\"id\": \"r1\"}\n\n:{}")]
#[case::default_order(RESPONSE, &[], Look::Kinds, "log:Debug|logger_fn|logger_post")]
#[case::sentry_precedes_the_logger(RESPONSE, &[Knob::Sentry], Look::Kinds, "log:Debug|logger_fn|sentry|logger_post")]
#[case::logger_gets_the_call_start_time(RESPONSE, &[], Look::Mentions(LOGGER_POST, "post_api_call"), "yes")]
#[case::logger_fn_failure_is_swallowed(RESPONSE, &[Knob::FailingLoggerFn], Look::Kinds, "log:Debug|logger_fn|log:Error|logger_post")]
#[case::missing_logger_fn_is_skipped(RESPONSE, &[Knob::NoLoggerFn], Look::Kinds, "log:Debug|logger_post")]
#[case::logger_fn_failure_is_logged(RESPONSE, &[Knob::FailingLoggerFn], Look::Record(1), "Error:LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging logger_fn exploded:{}")]
#[case::callback_failure_is_captured(RESPONSE, &[Knob::FailingCallback, Knob::Sentry], Look::Kinds, "log:Debug|logger_fn|sentry|logger_post|log:Error|captured")]
#[case::callback_failure_is_uncaptured_without_sentry(RESPONSE, &[Knob::FailingCallback], Look::Kinds, "log:Debug|logger_fn|logger_post|log:Error")]
#[case::callback_failure_is_logged(RESPONSE, &[Knob::FailingCallback], Look::Record(1), "Error:LiteLLM.LoggingError: [Non-Blocking] Exception occurred while post-call logging with integrations callback exploded:{}")]
#[case::breadcrumb_has_the_phase(RESPONSE, &[Knob::Sentry], Look::Mentions("sentry:Model Call Details post-call: ", ""), "yes")]
#[case::breadcrumb_keeps_the_input(RESPONSE, &[Knob::Sentry], Look::Mentions("sentry:", "hello"), "yes")]
#[case::breadcrumb_drops_the_input_when_logging_is_off(RESPONSE, &[Knob::Sentry, Knob::TurnOffMessages], Look::Mentions("sentry:", "hello"), "no")]
#[case::callback_sees_the_input(RESPONSE, &[], Look::Mentions(LOGGER_POST, "hello"), "yes")]
#[case::callback_gets_a_redacted_snapshot(RESPONSE, &[Knob::RedactContent], Look::Mentions(LOGGER_POST, "hello"), "no")]
#[case::logger_fn_gets_a_redacted_snapshot(RESPONSE, &[Knob::RedactContent], Look::Mentions("logger_fn:", "hello"), "no")]
#[case::state_keeps_the_raw_input(RESPONSE, &[Knob::RedactContent], Look::Detail(&["input"]), r#""hello""#)]
#[case::state_keeps_the_raw_response(RESPONSE, &[Knob::RedactContent], Look::Detail(&["original_response"]), r#""raw text""#)]
#[case::snapshot_redacts_the_response(RESPONSE, &[Knob::RedactContent], Look::Mentions("logger_post:", r#""original_response": "redacted-by-litellm""#), "yes")]
fn logging_pre_post_behavior_tests_behavior(
    #[case] call: &str,
    #[case] knobs: &[Knob],
    #[case] what: Look<'_>,
    #[case] expected: &str,
) {
    assert_eq!(look(&run_post("", call, knobs), &what), expected);
}
