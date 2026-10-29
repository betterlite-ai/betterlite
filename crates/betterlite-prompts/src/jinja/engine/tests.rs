#![cfg(test)]

use super::{raise_exception, strftime_now};
use chrono::{TimeZone, Utc};
use rstest::rstest;

#[derive(Clone, Copy)]
enum Scenario {
    RaiseExceptionDoesNotRaise,
    StrftimeFormats,
    StrftimeRejectsUnrecognizedSpecifier,
}

#[rstest]
#[case::raise_exception_does_not_raise(Scenario::RaiseExceptionDoesNotRaise)]
#[case::strftime_now_formats_with_injected_clock(Scenario::StrftimeFormats)]
#[case::strftime_now_rejects_unrecognized_specifier(Scenario::StrftimeRejectsUnrecognizedSpecifier)]
fn jinja_prompts_behavior_tests_behavior(#[case] scenario: Scenario) {
    let now = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    match scenario {
        Scenario::RaiseExceptionDoesNotRaise => {
            assert_eq!(raise_exception("boom".to_owned()), "Error message - boom");
        }
        Scenario::StrftimeFormats => {
            assert_eq!(strftime_now(now, "%Y-%m-%d".to_owned()).unwrap(), "2024-01-02");
        }
        Scenario::StrftimeRejectsUnrecognizedSpecifier => {
            assert!(strftime_now(now, "%Q".to_owned()).is_err());
        }
    }
}
