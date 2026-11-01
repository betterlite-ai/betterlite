use chrono::{NaiveDate, NaiveTime};

const MONTHNAMES: [&str; 24] = [
    "jan",
    "feb",
    "mar",
    "apr",
    "may",
    "jun",
    "jul",
    "aug",
    "sep",
    "oct",
    "nov",
    "dec",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const DAYNAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

// Ports `email.utils.parsedate_to_datetime`, lenient enough to accept a mismatched weekday or a missing timezone.
pub(super) fn parse_date(raw: &str) -> String {
    if let Some(formatted) = parsed_fields(raw).and_then(format_fields) {
        return formatted;
    }
    raw.to_owned()
}

#[derive(Clone, Copy)]
struct Fields {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn format_fields(fields: Fields) -> Option<String> {
    if !(1_i32..=9_999_i32).contains(&fields.year) {
        return None;
    }
    NaiveTime::from_hms_opt(fields.hour, fields.minute, fields.second)?;
    let date = NaiveDate::from_ymd_opt(fields.year, fields.month, fields.day)?;
    Some(date.format("%Y-%m-%d").to_string())
}

fn parsed_fields(raw: &str) -> Option<Fields> {
    let mut tokens: Vec<String> = raw.split_whitespace().map(str::to_owned).collect();
    strip_dayname(&mut tokens);
    expand_rfc850(&mut tokens);
    split_glued_timezone(&mut tokens);
    if tokens.len() < 5 {
        return None;
    }
    tokens.truncate(5);
    let fields: [String; 5] = tokens.try_into().ok()?;
    let [mut day, mut month, mut year, mut time, mut zone] = fields;
    if day.is_empty() || month.is_empty() || year.is_empty() {
        return None;
    }
    let month_number = resolve_month(&mut day, &mut month)?;
    strip_trailing_comma(&mut day);
    reorder_year_time_zone(&mut year, &mut time, &mut zone)?;
    strip_trailing_comma(&mut time);
    let (hour, minute, second) = split_time(&time)?;
    if !valid_timezone(&zone) {
        return None;
    }
    let parsed_year = normalize_year(year.parse().ok()?)?;
    let parsed_day = day.parse().ok()?;
    Some(Fields {
        year: parsed_year,
        month: month_number,
        day: parsed_day,
        hour,
        minute,
        second,
    })
}

// Ports `email._parseaddr._parsedate_tz`'s zone handling: an unparsable token means no zone.
fn valid_timezone(zone: &str) -> bool {
    let Ok(offset) = zone.parse::<i64>() else {
        return true;
    };
    let magnitude = offset.unsigned_abs();
    let Some(hours) = magnitude.checked_div(100) else {
        return false;
    };
    let Some(minutes) = magnitude.checked_rem(100) else {
        return false;
    };
    let Some(hour_seconds) = hours.checked_mul(3600) else {
        return false;
    };
    let Some(minute_seconds) = minutes.checked_mul(60) else {
        return false;
    };
    let Some(seconds) = hour_seconds.checked_add(minute_seconds) else {
        return false;
    };
    seconds < 86_400
}

fn strip_trailing_comma(token: &mut String) {
    if let Some(stripped) = token.strip_suffix(',') {
        stripped.to_owned().clone_into(token);
    }
}

fn strip_dayname(tokens: &mut Vec<String>) {
    let Some(first) = tokens.first().cloned() else {
        return;
    };
    if first.ends_with(',') || DAYNAMES.contains(&first.to_lowercase().as_str()) {
        tokens.remove(0);
        return;
    }
    let Some(position) = first.rfind(',') else {
        return;
    };
    let Some(start) = position.checked_add(1) else {
        return;
    };
    let Some(rest) = first.get(start..) else {
        return;
    };
    if let Some(slot) = tokens.get_mut(0) {
        rest.to_owned().clone_into(slot);
    }
}

fn expand_rfc850(tokens: &mut Vec<String>) {
    if tokens.len() != 3 {
        return;
    }
    let Some(first) = tokens.first() else {
        return;
    };
    let pieces: Vec<&str> = first.split('-').collect();
    if pieces.len() != 3 {
        return;
    }
    let mut expanded: Vec<String> = pieces.into_iter().map(str::to_owned).collect();
    let rest: Vec<String> = tokens.get(1..).map(<[String]>::to_vec).unwrap_or_default();
    expanded.extend(rest);
    *tokens = expanded;
}

fn split_glued_timezone(tokens: &mut Vec<String>) {
    if tokens.len() != 4 {
        return;
    }
    let Some(glued) = tokens.get(3).cloned() else {
        return;
    };
    let split_at = glued.find('+').or_else(|| glued.find('-'));
    match split_at {
        Some(position) if position > 0 => {
            let Some((before, after)) = glued.split_at_checked(position) else {
                return;
            };
            if let Some(last) = tokens.get_mut(3) {
                before.to_owned().clone_into(last);
            }
            tokens.push(after.to_owned());
        }
        _ => tokens.push(String::new()),
    }
}

fn resolve_month(day: &mut String, month: &mut String) -> Option<u32> {
    let lowered = month.to_lowercase();
    if MONTHNAMES.contains(&lowered.as_str()) {
        lowered.clone_into(month);
    } else {
        std::mem::swap(month, day);
        *month = month.to_lowercase();
        if !MONTHNAMES.contains(&month.as_str()) {
            return None;
        }
    }
    month_number_from_name(month)
}

fn month_number_from_name(name: &str) -> Option<u32> {
    let position = MONTHNAMES.iter().position(|month| *month == name)?;
    let one_based = position.checked_add(1)?;
    let month_number = u32::try_from(one_based).ok()?;
    if month_number > 12 {
        month_number.checked_sub(12)
    } else {
        Some(month_number)
    }
}

fn reorder_year_time_zone(year: &mut String, time: &mut String, zone: &mut String) -> Option<()> {
    if year.find(':').is_some_and(|position| position > 0) {
        std::mem::swap(year, time);
    }
    if let Some(stripped) = year.strip_suffix(',') {
        if stripped.is_empty() {
            return None;
        }
        stripped.to_owned().clone_into(year);
    }
    if !year.starts_with(|character: char| character.is_ascii_digit()) {
        std::mem::swap(year, zone);
    }
    Some(())
}

fn split_time(time: &str) -> Option<(u32, u32, u32)> {
    let colon_parts: Vec<&str> = time.split(':').collect();
    let (hour, minute, second) = match colon_parts.as_slice() {
        [hour, minute] => (*hour, *minute, "0"),
        [hour, minute, second] => (*hour, *minute, *second),
        [single] if single.contains('.') => {
            let dot_parts: Vec<&str> = single.split('.').collect();
            match dot_parts.as_slice() {
                [hour, minute] => (*hour, *minute, "0"),
                [hour, minute, second] => (*hour, *minute, *second),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some((
        hour.parse().ok()?,
        minute.parse().ok()?,
        second.parse().ok()?,
    ))
}

const fn normalize_year(year: i32) -> Option<i32> {
    if year >= 100_i32 {
        return Some(year);
    }
    if year > 68_i32 {
        year.checked_add(1900_i32)
    } else {
        year.checked_add(2000_i32)
    }
}

#[path = "date_parse/tests.rs"]
mod tests;
