use chrono::NaiveTime;
use thiserror::Error;

use super::schema::{HoursField, StringOrIgnored};

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    #[error("off-peak hours_utc is not a string or a list of strings")]
    NotIterable,
}

pub(super) fn window_strings(field: Option<&HoursField>) -> Vec<String> {
    match field {
        Some(HoursField::One(text)) => vec![text.clone()],
        Some(HoursField::Many(items)) => items
            .iter()
            .filter_map(|item| match item {
                StringOrIgnored::Text(text) => Some(text.clone()),
                StringOrIgnored::Ignored(_) => None,
            })
            .collect(),
        Some(HoursField::Other(_)) | None => Vec::new(),
    }
}

pub fn is_within_off_peak_window(
    field: &HoursField,
    moment: NaiveTime,
) -> Result<bool, WindowError> {
    match field {
        HoursField::One(text) => Ok(window_contains(text, moment)),
        HoursField::Many(items) => Ok(items.iter().any(|item| match item {
            StringOrIgnored::Text(text) => window_contains(text, moment),
            StringOrIgnored::Ignored(_) => false,
        })),
        HoursField::Other(_) => Err(WindowError::NotIterable),
    }
}

pub(super) fn matches_any_window(windows: &[String], moment: NaiveTime) -> bool {
    for window in windows {
        if window_contains(window, moment) {
            return true;
        }
    }
    false
}

fn window_contains(window: &str, moment: NaiveTime) -> bool {
    let Some((start, end)) = parse_window(window) else {
        return false;
    };
    if start < end {
        start <= moment && moment < end
    } else {
        moment >= start || moment < end
    }
}

fn parse_window(window: &str) -> Option<(NaiveTime, NaiveTime)> {
    let mut parts = window.split('-');
    let start_text = parts.next()?;
    let end_text = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((
        parse_clock(start_text.trim())?,
        parse_clock(end_text.trim())?,
    ))
}

fn parse_clock(text: &str) -> Option<NaiveTime> {
    let mut parts = text.split(':');
    let hour_text = parts.next()?;
    let minute_text = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if !is_one_or_two_digits(hour_text) || !is_one_or_two_digits(minute_text) {
        return None;
    }
    let hour: u32 = hour_text.parse().ok()?;
    let minute: u32 = minute_text.parse().ok()?;
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn is_one_or_two_digits(text: &str) -> bool {
    if !matches!(text.len(), 1 | 2) {
        return false;
    }
    text.bytes().all(|byte| byte.is_ascii_digit())
}
