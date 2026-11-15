use chrono::{NaiveDate, NaiveDateTime, TimeDelta};

use crate::key_attribution::history::window::HistoryWindow;
use crate::key_attribution::metadata::KeyMetadata;

pub(super) const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(super) const OTHER_DIGEST: &str =
    "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

pub(super) fn at(second: i64) -> NaiveDateTime {
    let base = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    base + TimeDelta::try_seconds(second).unwrap()
}

pub(super) fn window(start: i64, end: i64) -> HistoryWindow {
    HistoryWindow {
        start: at(start),
        end: at(end),
    }
}

pub(super) fn named(alias: &str) -> KeyMetadata {
    KeyMetadata {
        key_alias: Some(alias.to_owned()),
        team_id: None,
        user_id: None,
        user_email: None,
        key_exists: None,
    }
}
