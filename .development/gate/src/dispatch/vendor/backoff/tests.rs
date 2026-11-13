#![cfg(test)]

use std::time::Duration;

use super::{reset, waiting};

#[test]
fn authentication_and_transient_network_errors_have_different_recovery() {
    let age = Duration::from_secs(61);
    let fallback = Duration::from_mins(30);
    assert!(waiting("Not logged in", age, fallback, 0));
    assert!(!waiting(
        "Can't reach the API server (EAI_AGAIN)",
        age,
        fallback,
        0
    ));
    assert!(waiting("You've hit your usage limit", age, fallback, 0));
    assert!(waiting(
        "API Error: 401 Authentication Fails",
        Duration::from_hours(2),
        fallback,
        0
    ));
}

#[test]
fn a_reported_reset_overrides_the_thirty_minute_retry() {
    let message = "You've hit your weekly limit \u{b7} resets Nov 8, 2026 7am (Europe/Berlin)";
    let deadline = reset(message, 0).unwrap();
    assert!(waiting(
        message,
        Duration::from_secs(3600),
        Duration::from_mins(30),
        deadline - 1
    ));
    assert!(!waiting(
        message,
        Duration::ZERO,
        Duration::from_mins(30),
        deadline
    ));
}

#[test]
fn codex_ordinal_dates_parse_without_losing_the_reset_time() {
    let message = "ERROR: You've hit your usage limit. try again at Nov 6th, 2026 10:23 PM.";
    assert!(reset(message, 0).is_some());
    assert_eq!(reset("You've hit your usage limit", 0), None);
}

#[test]
fn a_yearless_january_reset_uses_the_recorded_december_marker() {
    let observed = super::date("2026-12-31 20:00 UTC", "%s", None)
        .unwrap()
        .parse()
        .unwrap();
    let expected = super::date("2027-01-02 07:00 UTC", "%s", None)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(reset("resets Jan 2, 7am (UTC)", observed), Some(expected));
}

#[test]
fn the_watchdogs_daily_ceiling_mark_holds_deepseek_until_midnight() {
    let message = "DeepSeek spent 301.20 USD today at list price, the 300 USD daily ceiling; resets 12am (Europe/Berlin)";
    let observed = super::date("2026-11-13 14:00", "%s", Some("Europe/Berlin"))
        .unwrap()
        .parse()
        .unwrap();
    let midnight = super::date("2026-11-14 00:00", "%s", Some("Europe/Berlin"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(reset(message, observed), Some(midnight));
    let fallback = Duration::from_mins(30);
    assert!(waiting(message, Duration::ZERO, fallback, midnight - 1));
    assert!(!waiting(message, Duration::ZERO, fallback, midnight));
}
