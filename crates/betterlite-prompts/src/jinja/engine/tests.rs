#![cfg(test)]

use super::strftime_now;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;

#[derive(Clone, Copy)]
enum Scenario {
    StrftimeFormats,
    StrftimeRejectsUnrecognizedSpecifier,
}

#[rstest]
#[case::strftime_now_formats_with_injected_clock(Scenario::StrftimeFormats)]
#[case::strftime_now_rejects_unrecognized_specifier(Scenario::StrftimeRejectsUnrecognizedSpecifier)]
fn jinja_prompts_behavior_tests_behavior(#[case] scenario: Scenario) {
    let now = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    match scenario {
        Scenario::StrftimeFormats => {
            assert_eq!(strftime_now(now, "%Y-%m-%d").unwrap(), "2024-01-02");
        }
        Scenario::StrftimeRejectsUnrecognizedSpecifier => {
            strftime_now(now, "%Q").unwrap_err();
        }
    }
}
