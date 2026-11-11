use chrono::{DateTime, Datelike as _, NaiveDateTime, Utc};

// int(timestamp) truncates toward zero, so a pre-1970 moment with a fraction moves up a second
pub(crate) const fn epoch_seconds(moment: DateTime<Utc>) -> i64 {
    let seconds = moment.timestamp();
    if seconds < 0 && moment.timestamp_subsec_nanos() > 0 {
        return seconds.saturating_add(1);
    }
    seconds
}

// LiteLLM feeds datetime.fromisoformat the str() of the value with every Z spelled as +00:00
#[must_use]
pub fn parse_iso_timestamp(text: &str) -> Option<i64> {
    if text.is_empty() {
        return None;
    }
    let normalized = text.replace('Z', "+00:00").replace(',', ".");
    let canonical = canonical_iso(&normalized)?;
    if let Ok(moment) = DateTime::parse_from_rfc3339(&canonical) {
        return Some(epoch_seconds(moment.with_timezone(&Utc)));
    }
    let naive = NaiveDateTime::parse_from_str(&canonical, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    // a naive timestamp checks the previous local day, so CPython cannot take 0001-01-01 itself
    if naive.year() == 1_i32 && (naive.month(), naive.day()) < (1_u32, 2_u32) {
        return None;
    }
    Some(epoch_seconds(naive.and_utc()))
}

// fromisoformat's extended grammar; a lowercase z, whitespace past the separator, the leap second and year 0 are rejected
fn canonical_iso(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let date = date_text(bytes)?;
    let Some(separator) = bytes.get(10) else {
        return Some(format!("{date}T00:00:00"));
    };
    if *separator != b'T' && *separator != b' ' {
        return None;
    }
    let time = time_text(bytes.get(11..)?)?;
    Some(format!("{date}T{time}"))
}

fn date_text(bytes: &[u8]) -> Option<String> {
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = number(bytes, 0, 4)?;
    let month = number(bytes, 5, 2)?;
    let day = number(bytes, 8, 2)?;
    if year == 0 {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn time_text(bytes: &[u8]) -> Option<String> {
    let hour = number(bytes, 0, 2)?;
    let Some(separator) = bytes.get(2) else {
        return Some(format!("{hour:02}:00:00"));
    };
    match *separator {
        b':' => {
            let minute = number(bytes, 3, 2)?;
            let (second, fraction, rest) = seconds(bytes.get(5..)?)?;
            let offset = if rest.is_empty() {
                String::new()
            } else {
                offset_text(rest, 0)?
            };
            Some(format!("{hour:02}:{minute:02}:{second}{fraction}{offset}"))
        }
        b'+' | b'-' => {
            let offset = offset_text(bytes, 2)?;
            Some(format!("{hour:02}:00:00{offset}"))
        }
        _ => None,
    }
}

fn seconds(bytes: &[u8]) -> Option<(String, String, &[u8])> {
    if bytes.first() != Some(&b':') {
        return Some(("00".to_owned(), String::new(), bytes));
    }
    let second = number(bytes, 1, 2)?;
    if second == 60 {
        return None;
    }
    let (fraction, rest) = fraction_text(bytes.get(3..)?)?;
    Some((format!("{second:02}"), fraction, rest))
}

fn fraction_text(bytes: &[u8]) -> Option<(String, &[u8])> {
    if bytes.first() != Some(&b'.') {
        return Some((String::new(), bytes));
    }
    let after = bytes.get(1..)?;
    let digits = digit_run(after);
    if digits.is_empty() {
        return None;
    }
    let kept = digits.get(..digits.len().min(6))?;
    let mut fraction = String::from(".");
    fraction.push_str(str::from_utf8(kept).ok()?);
    Some((fraction, after.get(digits.len()..)?))
}

fn digit_run(bytes: &[u8]) -> &[u8] {
    let mut count = 0_usize;
    while bytes.get(count).is_some_and(u8::is_ascii_digit) {
        count = count.saturating_add(1);
    }
    bytes.get(..count).unwrap_or_default()
}

fn offset_text(bytes: &[u8], at: usize) -> Option<String> {
    let sign = *bytes.get(at)?;
    let mark = match sign {
        b'+' => '+',
        b'-' => '-',
        _ => return None,
    };
    let hour_at = at.checked_add(1)?;
    let hour = number(bytes, hour_at, 2)?;
    let rest = bytes.get(hour_at.checked_add(2)?..)?;
    let minute = match rest {
        [] => 0,
        [b':', _, _] => number(rest, 1, 2)?,
        [_, _] => number(rest, 0, 2)?,
        _ => return None,
    };
    Some(format!("{mark}{hour:02}:{minute:02}"))
}

fn number(bytes: &[u8], at: usize, count: usize) -> Option<u32> {
    let end = at.checked_add(count)?;
    let mut value = 0_u32;
    for digit in bytes.get(at..end)? {
        if !digit.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(digit.checked_sub(b'0')?))?;
    }
    Some(value)
}
