#![cfg(test)]

use rstest::rstest;

use crate::datadog_llm_privacy::redaction_decision::{
    LoggingFlags, REDACTS_MESSAGES_ITSELF, payload_logging_is_off,
};

#[rstest]
#[case::global_flag_alone((true, true, false), true)]
#[case::instance_flag_alone((false, false, false), true)]
#[case::per_request_alone((false, true, true), true)]
#[case::nothing_set((false, true, false), false)]
#[case::global_flag_and_per_request((true, true, true), true)]
#[case::instance_flag_and_per_request((false, false, true), true)]
fn payload_logging_is_off_ors_the_three_signals(
    #[case] setup: (bool, bool, bool),
    #[case] expected: bool,
) {
    let flags = LoggingFlags {
        turn_off_message_logging: setup.0,
        message_logging: setup.1,
    };
    assert_eq!(payload_logging_is_off(flags, setup.2), expected);
}

#[test]
fn the_logger_redacts_messages_itself() {
    assert_eq!(REDACTS_MESSAGES_ITSELF.to_string(), "true");
}
