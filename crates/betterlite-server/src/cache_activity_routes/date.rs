use chrono::{DateTime, NaiveDate, NaiveTime, Utc};

// Python's strptime("%Y-%m-%d"): a four digit year, a one or two digit month and day,
// a space in place of a one digit day's zero, and no text or whitespace around it.
pub(crate) fn parse_date(value: &str) -> Option<DateTime<Utc>> {
    let (year_text, remainder) = value.split_once('-')?;
    let (month_text, day_text) = remainder.split_once('-')?;
    let year = parse_year(year_text)?;
    let month = parse_bounded_number(month_text, 12)?;
    let day = parse_day(day_text)?;
    NaiveDate::from_ymd_opt(year, month, day).map(|date| date.and_time(NaiveTime::MIN).and_utc())
}

fn parse_year(value: &str) -> Option<i32> {
    if value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = value.parse::<i32>().ok()?;
    (year >= 1_i32).then_some(year)
}

fn parse_day(value: &str) -> Option<u32> {
    let Some(single) = value.strip_prefix(' ') else {
        return parse_bounded_number(value, 31);
    };
    match single.as_bytes() {
        [digit @ b'1'..=b'9'] => char::from(*digit).to_digit(10),
        _ => None,
    }
}

fn parse_bounded_number(value: &str, maximum: u32) -> Option<u32> {
    if value.is_empty() || value.len() > 2 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = value.parse::<u32>().ok()?;
    (1_u32..=maximum).contains(&number).then_some(number)
}

// CPython's repr(str): single quotes unless the text holds a single quote and no double
// quote, then double quotes; backslash, newline, tab, carriage return and control
// characters are escaped.
pub(crate) fn python_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::new();
    rendered.push(quote);
    for character in value.chars() {
        push_escaped(&mut rendered, character, quote);
    }
    rendered.push(quote);
    rendered
}

fn push_escaped(rendered: &mut String, character: char, quote: char) {
    match character {
        '\\' => rendered.push_str("\\\\"),
        '\n' => rendered.push_str("\\n"),
        '\r' => rendered.push_str("\\r"),
        '\t' => rendered.push_str("\\t"),
        '\'' if quote == '\'' => {
            rendered.push('\\');
            rendered.push('\'');
        }
        control if control < ' ' || matches!(control, '\u{7f}'..='\u{9f}') => {
            let escaped = format!("\\x{:02x}", u32::from(control));
            rendered.push_str(&escaped);
        }
        other => rendered.push(other),
    }
}
