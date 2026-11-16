#![cfg(test)]

use chrono::{DateTime, Utc};
use rstest::rstest;
use tracing::subscriber::set_default;

use super::support::{at, capture_logs, dates, window};
use super::window::spend_logs_window;

#[rstest]
#[case("2026-01-05", at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))]
#[case("20260105", at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))]
#[case("2026-01-05T10:30:00", at(2026, 1, 4, 10, 30), at(2026, 1, 7, 10, 30))]
#[case("2026-01-05 10:30:00", at(2026, 1, 4, 10, 30), at(2026, 1, 7, 10, 30))]
#[case("2026-01-05T10:30:00Z", at(2026, 1, 4, 10, 30), at(2026, 1, 7, 10, 30))]
#[case(
    "2026-01-05T10:30:00+02:00",
    at(2026, 1, 4, 8, 30),
    at(2026, 1, 7, 8, 30)
)]
fn parse_spend_date_accepts_iso_datetimes(
    #[case] raw: &str,
    #[case] start: DateTime<Utc>,
    #[case] end: DateTime<Utc>,
) {
    assert_eq!(
        spend_logs_window(&dates(&[raw])).unwrap(),
        window(start, end)
    );
}

#[rstest]
#[case("")]
#[case("not-a-date")]
#[case("2026-13-01")]
#[case("2026-01-32")]
#[case("2026-02-30")]
#[case("05-01-2026")]
#[case("2026/01/05")]
#[case("2026-01-05T25:00:00")]
#[case("2026-01-05Z10:30:00")]
fn parse_spend_date_returns_none_for_invalid_input(#[case] raw: &str) {
    assert!(spend_logs_window(&dates(&[raw])).is_none());
}

#[test]
fn parse_spend_date_returns_none_without_logging() {
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let computed = spend_logs_window(&dates(&["", "not-a-date", "2026-02-30"]));
    drop(guard);
    assert!(computed.is_none());
    assert!(logs.contents().is_empty());
}
