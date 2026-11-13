use rstest::rstest;

use super::support::{CALL, Knob, Look, look, run_pre};

const MESSAGES: &str = r#","messages":[{"role":"user","content":"secret question"}]"#;
const ONLY_RAW: &str = r#","request_logging":{"raw_request_only":true}"#;
const OK: &str = "Ok(PreCallOutput { classifier_input: None })";

#[rstest]
#[case::default_order("", &[], Look::Kinds, "logger_fn|logger_pre|input_event")]
#[case::sentry_runs_first("", &[Knob::Sentry], Look::Kinds, "logger_fn|sentry|logger_pre|input_event")]
#[case::debug_then_raw_then_hooks("", &[Knob::Verbose, Knob::Sentry, Knob::GlobalRawLogging], Look::Kinds, "log:Debug|logger_fn|sentry|logger_pre|input_event")]
#[case::logger_fn_failure_is_swallowed("", &[Knob::FailingLoggerFn], Look::Kinds, "logger_fn|log:Error|logger_pre|input_event")]
#[case::logger_fn_failure_is_logged("", &[Knob::FailingLoggerFn], Look::Record(0), "Error:LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging logger_fn exploded:{}")]
#[case::logger_fn_failure_still_succeeds("", &[Knob::FailingLoggerFn], Look::Outcome, OK)]
#[case::missing_logger_fn_is_skipped("", &[Knob::NoLoggerFn], Look::Kinds, "logger_pre|input_event")]
#[case::callback_failure_is_isolated("", &[Knob::FailingCallback], Look::Kinds, "logger_fn|logger_pre|log:Error|input_event")]
#[case::callback_failure_is_captured("", &[Knob::FailingCallback, Knob::Sentry], Look::Kinds, "logger_fn|sentry|logger_pre|log:Error|captured|input_event")]
#[case::callback_failure_is_logged("", &[Knob::FailingCallback], Look::Record(0), "Error:litellm.Logging.pre_call(): Exception occurred - callback exploded:{}")]
#[case::callback_failure_still_succeeds("", &[Knob::FailingCallback], Look::Outcome, OK)]
#[case::raw_request_only_raises_after_callbacks(ONLY_RAW, &[], Look::Outcome, "Err(RawRequestCaptured)")]
#[case::raw_request_only_runs_every_callback(ONLY_RAW, &[], Look::Kinds, "logger_fn|logger_pre|input_event")]
#[case::breadcrumb_has_the_phase("", &[Knob::Sentry], Look::Mentions("sentry:Model Call Details pre-call: ", ""), "yes")]
#[case::breadcrumb_keeps_messages("", &[Knob::Sentry], Look::Mentions("sentry:", "call input"), "yes")]
#[case::breadcrumb_drops_messages_when_logging_is_off("", &[Knob::Sentry, Knob::TurnOffMessages], Look::Mentions("sentry:", "call input"), "no")]
#[case::logger_fn_sees_the_input("", &[], Look::Mentions("logger_fn:", "call input"), "yes")]
#[case::logger_fn_gets_a_redacted_snapshot("", &[Knob::RedactContent], Look::Mentions("logger_fn:", "call input"), "no")]
#[case::callback_sees_the_messages(MESSAGES, &[], Look::Mentions("logger_pre:", "secret question"), "yes")]
#[case::callback_gets_redacted_messages(MESSAGES, &[Knob::RedactContent], Look::Mentions("logger_pre:", "secret question"), "no")]
#[case::state_keeps_the_raw_input("", &[Knob::RedactContent], Look::Detail(&["input"]), r#"[{"role": "user", "content": "call input"}]"#)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] extra: &str,
    #[case] knobs: &[Knob],
    #[case] what: Look<'_>,
    #[case] expected: &str,
) {
    assert_eq!(look(&run_pre(extra, CALL, knobs), &what), expected);
}
