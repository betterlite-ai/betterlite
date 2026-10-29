use std::collections::BTreeSet;

use chrono::{DateTime, Datelike as _, Utc};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::schema::{TimezoneField, WeekdayEntry, WeekdaysField};

pub(super) fn matches_weekdays(
    moment: DateTime<Utc>,
    weekdays: Option<&WeekdaysField>,
    weekday_timezone: Option<&TimezoneField>,
) -> bool {
    let Some(weekdays_field) = weekdays else {
        return true;
    };
    let WeekdaysField::List(entries) = weekdays_field else {
        return false;
    };
    let allowed: BTreeSet<u32> = entries.iter().filter_map(normalize_weekday).collect();
    allowed.contains(&iso_weekday(moment, weekday_timezone))
}

fn normalize_weekday(entry: &WeekdayEntry) -> Option<u32> {
    match entry {
        WeekdayEntry::Bool(_) | WeekdayEntry::Other(_) => None,
        WeekdayEntry::Int(value) => u32::try_from(*value)
            .ok()
            .filter(|day| (1..=7).contains(day)),
        WeekdayEntry::Text(text) => named_weekday(text.trim()),
    }
}

fn named_weekday(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(1),
        "tue" | "tues" | "tuesday" => Some(2),
        "wed" | "wednesday" => Some(3),
        "thu" | "thur" | "thurs" | "thursday" => Some(4),
        "fri" | "friday" => Some(5),
        "sat" | "saturday" => Some(6),
        "sun" | "sunday" => Some(7),
        _ => None,
    }
}

fn iso_weekday(moment: DateTime<Utc>, weekday_timezone: Option<&TimezoneField>) -> u32 {
    let name = match weekday_timezone {
        Some(TimezoneField::Text(text)) => text.trim(),
        _ => "",
    };
    if name.is_empty() {
        return moment.weekday().number_from_monday();
    }
    let Ok(zone) = TimeZone::get(name) else {
        return moment.weekday().number_from_monday();
    };
    let Ok(nanosecond) = i32::try_from(moment.timestamp_subsec_nanos()) else {
        return moment.weekday().number_from_monday();
    };
    let Ok(timestamp) = Timestamp::new(moment.timestamp(), nanosecond) else {
        return moment.weekday().number_from_monday();
    };
    u32::try_from(zone.to_datetime(timestamp).weekday().to_monday_one_offset())
        .unwrap_or_else(|_| moment.weekday().number_from_monday())
}
