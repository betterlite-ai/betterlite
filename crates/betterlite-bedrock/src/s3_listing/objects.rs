use chrono::NaiveDate;

use super::S3ListingError;

struct Cursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Cursor<'_> {
    fn digit(&mut self) -> Option<u32> {
        let byte = *self.bytes.get(self.index)?;
        let digit = ascii_digit(byte)?;
        self.index = self.index.saturating_add(1);
        Some(digit)
    }

    fn digits(&mut self, count: usize) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..count {
            let digit = self.digit()?;
            value = value.saturating_mul(10).saturating_add(digit);
        }
        Some(value)
    }
}

fn ascii_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte.wrapping_sub(b'0'))),
        _ => None,
    }
}

fn read_date_parts(cursor: &mut Cursor<'_>) -> Option<(u32, u32)> {
    if cursor.bytes.get(cursor.index) == Some(&b'-') {
        cursor.index = cursor.index.saturating_add(1);
        let month = cursor.digits(2)?;
        if cursor.bytes.get(cursor.index) != Some(&b'-') {
            return None;
        }
        cursor.index = cursor.index.saturating_add(1);
        let day = cursor.digits(2)?;
        return Some((month, day));
    }
    let month = cursor.digits(2)?;
    let day = cursor.digits(2)?;
    Some((month, day))
}

fn read_fraction(cursor: &mut Cursor<'_>) -> Option<u32> {
    match cursor.bytes.get(cursor.index).copied() {
        Some(b'.' | b',') => cursor.index = cursor.index.saturating_add(1),
        Some(_) | None => return Some(0),
    }
    let mut micros = 0_u32;
    let mut count = 0_usize;
    while let Some(byte) = cursor.bytes.get(cursor.index).copied() {
        let Some(digit) = ascii_digit(byte) else {
            break;
        };
        if count < 6 {
            micros = micros.saturating_mul(10).saturating_add(digit);
            count = count.saturating_add(1);
        }
        cursor.index = cursor.index.saturating_add(1);
    }
    if count == 0 {
        return None;
    }
    while count < 6 {
        micros = micros.saturating_mul(10);
        count = count.saturating_add(1);
    }
    Some(micros)
}

fn read_time(cursor: &mut Cursor<'_>) -> Option<(u32, u32, u32, u32)> {
    let hour = cursor.digits(2)?;
    let mut minute = 0_u32;
    let mut second = 0_u32;
    let mut micros = 0_u32;
    match cursor.bytes.get(cursor.index).copied() {
        Some(b':') => {
            cursor.index = cursor.index.saturating_add(1);
            minute = cursor.digits(2)?;
            if cursor.bytes.get(cursor.index) == Some(&b':') {
                cursor.index = cursor.index.saturating_add(1);
                second = cursor.digits(2)?;
                micros = read_fraction(cursor)?;
            }
        }
        Some(byte) if ascii_digit(byte).is_some() => {
            minute = cursor.digits(2)?;
            let basic_seconds = cursor
                .bytes
                .get(cursor.index)
                .copied()
                .and_then(ascii_digit)
                .is_some();
            if basic_seconds {
                second = cursor.digits(2)?;
                micros = read_fraction(cursor)?;
            }
        }
        Some(_) | None => {}
    }
    Some((hour, minute, second, micros))
}

fn read_offset(cursor: &mut Cursor<'_>) -> Option<i64> {
    let sign = match cursor.bytes.get(cursor.index).copied() {
        Some(b'-') => -1_i64,
        Some(b'+') => 1_i64,
        Some(_) | None => return None,
    };
    cursor.index = cursor.index.saturating_add(1);
    let hours = cursor.digits(2)?;
    let mut minutes = 0_u32;
    let mut seconds = 0_u32;
    match cursor.bytes.get(cursor.index).copied() {
        Some(b':') => {
            cursor.index = cursor.index.saturating_add(1);
            minutes = cursor.digits(2)?;
            if cursor.bytes.get(cursor.index) == Some(&b':') {
                cursor.index = cursor.index.saturating_add(1);
                seconds = cursor.digits(2)?;
            }
        }
        Some(byte) if ascii_digit(byte).is_some() => minutes = cursor.digits(2)?,
        Some(_) | None => {}
    }
    if hours > 23 || minutes > 59 || seconds > 59 {
        return None;
    }
    let magnitude = i64::from(hours)
        .saturating_mul(3600)
        .saturating_add(i64::from(minutes).saturating_mul(60))
        .saturating_add(i64::from(seconds));
    Some(magnitude.saturating_mul(sign))
}

fn iso_timestamp(text: &str) -> Option<i64> {
    let mut cursor = Cursor {
        bytes: text.as_bytes(),
        index: 0,
    };
    let year = cursor.digits(4)?;
    if year == 0 {
        return None;
    }
    let (month, day) = read_date_parts(&mut cursor)?;
    let (hour, minute, second, micros) = if cursor.index >= cursor.bytes.len() {
        (0_u32, 0_u32, 0_u32, 0_u32)
    } else {
        cursor.index = cursor.index.saturating_add(1);
        read_time(&mut cursor)?
    };
    let offset = if cursor.index >= cursor.bytes.len() {
        0_i64
    } else {
        read_offset(&mut cursor)?
    };
    if cursor.index != cursor.bytes.len() {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day)?;
    let nanos = micros.saturating_mul(1_000);
    let naive = date.and_hms_nano_opt(hour, minute, second, nanos)?;
    let whole = naive.and_utc().timestamp();
    let shifted = whole.checked_sub(offset)?;
    if micros > 0 && shifted < 0 {
        shifted.checked_add(1)
    } else {
        Some(shifted)
    }
}

fn python_int(raw: &str) -> Option<i64> {
    let bytes = raw.trim().as_bytes();
    let (negative, digits) = match bytes.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        Some(_) => (false, bytes),
        None => return None,
    };
    let Some(first) = digits.first() else {
        return None;
    };
    if ascii_digit(*first).is_none() {
        return None;
    }
    let Some(last) = digits.last() else {
        return None;
    };
    if ascii_digit(*last).is_none() {
        return None;
    }
    let mut value = 0_i64;
    let mut underscore = false;
    for &byte in digits {
        match ascii_digit(byte) {
            Some(digit) => {
                value = value.checked_mul(10)?.checked_add(i64::from(digit))?;
                underscore = false;
            }
            None if byte == b'_' && !underscore => underscore = true,
            None => return None,
        }
    }
    if negative {
        value.checked_neg()
    } else {
        Some(value)
    }
}

// LiteLLM's int(entry.findtext("{*}Size") or 0): a missing or empty size is zero
#[must_use]
pub fn parse_size(value: Option<&str>) -> Result<i64, S3ListingError> {
    let Some(raw) = value else {
        return Ok(0);
    };
    if raw.is_empty() {
        return Ok(0);
    }
    match python_int(raw) {
        Some(size) => Ok(size),
        None => Err(S3ListingError::Size {
            text: raw.to_owned(),
        }),
    }
}

// LiteLLM's int(datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp())
#[must_use]
pub fn parse_created_at(value: Option<&str>) -> Result<i64, S3ListingError> {
    let Some(raw) = value else {
        return Ok(0);
    };
    if raw.is_empty() {
        return Ok(0);
    }
    let replaced = raw.replace('Z', "+00:00");
    match iso_timestamp(&replaced) {
        Some(stamp) => Ok(stamp),
        None => Err(S3ListingError::Timestamp {
            text: raw.to_owned(),
        }),
    }
}

#[must_use]
pub fn posix_basename(object_key: &str) -> String {
    match object_key.rsplit_once('/') {
        Some((_head, tail)) => tail.to_owned(),
        None => object_key.to_owned(),
    }
}
