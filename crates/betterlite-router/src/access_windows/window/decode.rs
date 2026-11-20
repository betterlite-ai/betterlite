use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use jiff::civil::Time;
use jiff::tz::{TimeZone, TimeZoneDatabase};

use super::{ModelAccessWindow, WindowError};

// The four keys ModelAccessWindow declares; pydantic forbids every other one.
const WINDOW_FIELDS: [&str; 4] = ["start", "end", "timezone", "team_ids"];

pub(super) fn window_at(
    index: usize,
    raw: &Json,
    database: &TimeZoneDatabase,
) -> Result<ModelAccessWindow, WindowError> {
    let Json::Object(entries) = raw else {
        return Err(WindowError::NotAnObject {
            location: index.to_string(),
        });
    };
    let start = time_field(entries, "start", index)?;
    let end = time_field(entries, "end", index)?;
    let (timezone, zone) = zone_field(entries, index, database)?;
    let team_ids = team_ids_field(entries, index)?;
    if let Some(field) = entries
        .keys()
        .find(|field| !WINDOW_FIELDS.contains(&field.as_str()))
    {
        return Err(WindowError::Extra {
            location: format!("{index}.{field}"),
        });
    }
    if start == end {
        return Err(WindowError::SameBounds {
            location: index.to_string(),
        });
    }
    Ok(ModelAccessWindow {
        start,
        end,
        timezone,
        zone,
        team_ids,
    })
}

fn time_field(entries: &JsonObject, field: &str, index: usize) -> Result<Time, WindowError> {
    let location = format!("{index}.{field}");
    match entries.get(field) {
        None | Some(Json::Null) => Err(WindowError::Missing { location }),
        Some(Json::Text(text)) => wall_clock(text, location),
        Some(_) => Err(WindowError::Time { location }),
    }
}

fn zone_field(
    entries: &JsonObject,
    index: usize,
    database: &TimeZoneDatabase,
) -> Result<(String, TimeZone), WindowError> {
    let location = format!("{index}.timezone");
    let name = match entries.get("timezone") {
        None | Some(Json::Null) => return Err(WindowError::Missing { location }),
        Some(Json::Text(text)) => text,
        Some(_) => return Err(WindowError::ZoneType { location }),
    };
    // Python's ZoneInfo matches the on-disk name case-sensitively, while jiff's lookup ignores ASCII case
    let found = database
        .get(name)
        .ok()
        .filter(|zone| zone.iana_name() == Some(name.as_str()));
    found.map_or_else(
        || {
            Err(WindowError::UnknownZone {
                location,
                name: name.clone(),
            })
        },
        |zone| Ok((name.clone(), zone)),
    )
}

fn team_ids_field(entries: &JsonObject, index: usize) -> Result<Vec<String>, WindowError> {
    let location = format!("{index}.team_ids");
    let Some(Json::List(items)) = entries
        .get("team_ids")
        .filter(|value| matches!(value, Json::List(items) if !items.is_empty()))
    else {
        return Err(WindowError::TeamIds { location });
    };
    let mut team_ids = Vec::with_capacity(items.len());
    for (position, item) in items.iter().enumerate() {
        let Json::Text(team_id) = item else {
            return Err(WindowError::TeamId {
                location: format!("{location}.{position}"),
            });
        };
        if team_id.is_empty() {
            return Err(WindowError::EmptyTeamId { location });
        }
        team_ids.push(team_id.clone());
    }
    Ok(team_ids)
}

// A UTC offset or Z suffix is what pydantic parses as an aware time, which the wall-clock check then refuses.
fn wall_clock(text: &str, location: String) -> Result<Time, WindowError> {
    if let Some(marker) = offset_marker(text) {
        let clock = text.get(..marker).unwrap_or(text);
        return if clock_shape(clock).is_some() {
            Err(WindowError::Offset { location })
        } else {
            Err(WindowError::Time { location })
        };
    }
    clock_shape(text).map_or_else(|| Err(WindowError::Time { location }), Ok)
}

fn offset_marker(text: &str) -> Option<usize> {
    text.char_indices()
        .skip(1)
        .find(|(_, character)| matches!(character, '+' | '-' | 'Z' | 'z'))
        .map(|(index, _)| index)
}

// HH:MM[:SS[.,ffffff]]: the zero-padded shape pydantic's time parser accepts, with at most microsecond fractions.
fn clock_shape(text: &str) -> Option<Time> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 {
        return None;
    }
    if bytes.get(2) != Some(&b':') {
        return None;
    }
    let hour = digits(bytes.get(..2)?)?;
    let minute = digits(bytes.get(3..5)?)?;
    if bytes.len() == 5 {
        return clock(hour, minute, 0, 0);
    }
    if bytes.get(5) != Some(&b':') {
        return None;
    }
    let second = digits(bytes.get(6..8)?)?;
    if bytes.len() == 8 {
        return clock(hour, minute, second, 0);
    }
    if !matches!(bytes.get(8), Some(&b'.' | &b',')) {
        return None;
    }
    clock(hour, minute, second, fraction(bytes.get(9..)?)?)
}

fn clock(hour: i32, minute: i32, second: i32, nanos: i32) -> Option<Time> {
    Time::new(
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        nanos,
    )
    .ok()
}

fn digits(bytes: &[u8]) -> Option<i32> {
    if bytes.len() != 2 {
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let tens = i32::from(bytes.first()?.checked_sub(b'0')?);
    let ones = i32::from(bytes.get(1)?.checked_sub(b'0')?);
    tens.checked_mul(10)?.checked_add(ones)
}

// A one-to-six digit fraction, scaled to the nanosecond field jiff's Time keeps.
fn fraction(bytes: &[u8]) -> Option<i32> {
    let scale = match bytes.len() {
        1 => 100_000_000_i32,
        2 => 10_000_000_i32,
        3 => 1_000_000_i32,
        4 => 100_000_i32,
        5 => 10_000_i32,
        6 => 1_000_i32,
        _ => return None,
    };
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value: i32 = 0;
    for digit in bytes {
        value = value
            .checked_mul(10)?
            .checked_add(i32::from(digit.checked_sub(b'0')?))?;
    }
    value.checked_mul(scale)
}
