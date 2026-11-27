use chrono::{Datelike as _, NaiveDate};

mod tests;

const MONTHS: [(&str, &str); 12] = [
    ("jan", "january"),
    ("feb", "february"),
    ("mar", "march"),
    ("apr", "april"),
    ("may", "may"),
    ("jun", "june"),
    ("jul", "july"),
    ("aug", "august"),
    ("sep", "september"),
    ("oct", "october"),
    ("nov", "november"),
    ("dec", "december"),
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
// email._parseaddr._timezones, in hours and minutes as the header writes them
const ZONES: [(&str, i64); 14] = [
    ("UT", 0),
    ("UTC", 0),
    ("GMT", 0),
    ("Z", 0),
    ("AST", -400),
    ("ADT", -300),
    ("EST", -500),
    ("EDT", -400),
    ("CST", -600),
    ("CDT", -500),
    ("MST", -700),
    ("MDT", -600),
    ("PST", -800),
    ("PDT", -700),
];
// Python's date(1970, 1, 1).toordinal(), which counts days the way chrono's num_days_from_ce does
const EPOCH_ORDINAL: i64 = 719_163;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedDate {
    pub year: i64,
    pub month: u32,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub zone_offset: Option<i64>,
}

// Python's int() takes a signed run of ASCII digits with single underscores between them; a magnitude past i64 saturates
#[must_use]
// ast-grep-ignore: no-python-habits - the task names Python's int() grammar the one port LiteLLM's date reader needs
pub fn python_int(text: &str) -> Option<i64> {
    let negative = text.starts_with('-');
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    if digits.is_empty() {
        return None;
    }
    let mut magnitude: u64 = 0;
    let mut previous_was_digit = false;
    for character in digits.chars() {
        if character == '_' {
            if !previous_was_digit {
                return None;
            }
            previous_was_digit = false;
            continue;
        }
        if !character.is_ascii_digit() {
            return None;
        }
        let digit = character.to_digit(10)?;
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u64::from(digit));
        previous_was_digit = true;
    }
    if !previous_was_digit {
        return None;
    }
    let limit = i64::MAX.unsigned_abs();
    if negative {
        if magnitude > limit {
            return Some(i64::MIN);
        }
        let value = i64::try_from(magnitude).ok()?;
        return Some(value.saturating_neg());
    }
    let value = i64::try_from(magnitude.min(limit)).ok()?;
    Some(value)
}

// email.utils.parsedate_tz splits on whitespace, drops a leading weekday unchecked and unpacks the RFC 850 form
fn date_tokens(header: &str) -> Option<Vec<&str>> {
    let mut words = header.split_whitespace();
    let first = words.next()?;
    let mut tokens: Vec<&str> = Vec::new();
    if !first.ends_with(',') && !WEEKDAYS.contains(&first.to_lowercase().as_str()) {
        tokens.push(first.rsplit_once(',').map_or(first, |(_, rest)| rest));
    }
    tokens.extend(words);
    if let [dated, second, third] = *tokens.as_slice() {
        let parts: Vec<&str> = dated.split('-').collect();
        if parts.len() == 3 {
            tokens = parts;
            tokens.extend([second, third]);
        }
    }
    if let [day, month, year, last] = *tokens.as_slice() {
        let split = last
            .find('+')
            .or_else(|| last.find('-'))
            .filter(|index| *index > 0)
            .and_then(|index| last.split_at_checked(index));
        let (time, zone) = split.unwrap_or((last, ""));
        tokens = vec![day, month, year, time, zone];
    }
    (tokens.len() >= 5).then_some(tokens)
}

// the fields may come in either order, as parsedate_tz swaps day and month, year and time, year and zone
#[must_use]
pub fn parsedate_tz(text: &str) -> Option<ParsedDate> {
    let tokens = date_tokens(text)?;
    let [day, month, year, clock, zone, ..] = *tokens.as_slice() else {
        return None;
    };
    let month_lower = month.to_lowercase();
    let (day_token, month_name) = if MONTHS
        .iter()
        .any(|names| names.0 == month_lower || names.1 == month_lower)
    {
        (day, month_lower)
    } else {
        (month, day.to_lowercase())
    };
    let month_index = MONTHS
        .iter()
        .position(|names| names.0 == month_name || names.1 == month_name)?;
    let day_text = day_token.strip_suffix(',').unwrap_or(day_token);
    let (year_order, clock_order) = if year.find(':').is_some_and(|index| index > 0) {
        (clock, year)
    } else {
        (year, clock)
    };
    let year_without_comma = year_order.strip_suffix(',').unwrap_or(year_order);
    if year_without_comma.is_empty() {
        return None;
    }
    let (year_text, zone_text) =
        if year_without_comma.starts_with(|first: char| first.is_ascii_digit()) {
            (year_without_comma, zone)
        } else {
            (zone, year_without_comma)
        };
    let clock_text = clock_order.strip_suffix(',').unwrap_or(clock_order);
    parsed_fields(year_text, month_index, day_text, clock_text, zone_text)
}

// parsedate_tz divides the time on colons, or on dots when it has no colon, and a two-part time holds no seconds
fn parsed_fields(
    year: &str,
    month_index: usize,
    day: &str,
    time: &str,
    zone: &str,
) -> Option<ParsedDate> {
    let separator = if time.contains(':') { ':' } else { '.' };
    let clock: Vec<&str> = time.split(separator).collect();
    let (hour_text, minute_text, second_text) = match *clock.as_slice() {
        [hour_text, minute_text] => (hour_text, minute_text, "0"),
        [hour_text, minute_text, second_text] => (hour_text, minute_text, second_text),
        _ => return None,
    };
    let written_year = python_int(year)?;
    // RFC 822 calls for a two-digit year and RFC 2822 mandates four digits, as POSIX reads the short form
    let year_value = if written_year < 100 {
        written_year.saturating_add(if written_year > 68 { 1900 } else { 2000 })
    } else {
        written_year
    };
    Some(ParsedDate {
        year: year_value,
        month: u32::try_from(month_index).ok()?.checked_add(1)?,
        day: python_int(day)?,
        hour: python_int(hour_text)?,
        minute: python_int(minute_text)?,
        second: python_int(second_text)?,
        zone_offset: zone_offset(zone),
    })
}

// calendar.timegm leaves day, hour, minute and second unchecked, so 31 Feb or 25:00 roll over as plain arithmetic
#[must_use]
pub fn mktime_tz(date: &ParsedDate) -> Option<i64> {
    let calendar_year = i32::try_from(date.year).ok()?;
    // datetime.date raises outside years 1 to 9999, and LiteLLM then ignores the header
    if !(1_i32..=9_999_i32).contains(&calendar_year) {
        return None;
    }
    let first_of_month = NaiveDate::from_ymd_opt(calendar_year, date.month, 1)?;
    let days = i64::from(first_of_month.num_days_from_ce())
        .saturating_sub(EPOCH_ORDINAL)
        .saturating_add(date.day)
        .saturating_sub(1);
    let hours = days.saturating_mul(24).saturating_add(date.hour);
    let minutes = hours.saturating_mul(60).saturating_add(date.minute);
    let seconds = minutes.saturating_mul(60).saturating_add(date.second);
    Some(seconds.saturating_sub(date.zone_offset.unwrap_or(0)))
}

// a zone that is neither a known name nor a number has no offset, and parsedate_tz turns its None into 0 at the caller
fn zone_offset(zone: &str) -> Option<i64> {
    let upper = zone.to_uppercase();
    let named = ZONES
        .iter()
        .find(|entry| entry.0 == upper)
        .map(|entry| entry.1);
    let written = named.or_else(|| python_int(zone))?;
    if written == 0 && zone.starts_with('-') {
        return None;
    }
    // Rust's division truncates toward zero, so a negative offset keeps Python's sign times magnitude
    let hours = written.checked_div(100)?.saturating_mul(3_600);
    let minutes = written.checked_rem(100)?.saturating_mul(60);
    Some(hours.saturating_add(minutes))
}
