use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use jiff::civil::Time;
use jiff::tz::{TimeZone, TimeZoneDatabase};
use thiserror::Error;

// The four keys ModelAccessWindow declares; pydantic forbids every other one.
const WINDOW_FIELDS: [&str; 4] = ["start", "end", "timezone", "team_ids"];

// ModelAccessWindow: a reservation is a local wall-clock span inside one named IANA zone, held by the teams it lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelAccessWindow {
    pub start: Time,
    pub end: Time,
    pub timezone: String,
    pub zone: TimeZone,
    pub team_ids: Vec<String>,
}

// Every way model_info.access_windows fails ModelAccessWindow's validation, located the way pydantic locates it.
#[derive(Debug, Error)]
pub enum WindowError {
    #[error("{location}: access_windows must be a list of windows")]
    NotAList { location: String },
    #[error("{location}: must be a window object")]
    NotAnObject { location: String },
    #[error("{location}: is required")]
    Missing { location: String },
    #[error("{location}: must be a time like 09:00")]
    Time { location: String },
    #[error("{location}: must be a local wall-clock time without a UTC offset")]
    Offset { location: String },
    #[error("{location}: must be an IANA timezone name")]
    ZoneType { location: String },
    #[error("{location}: unknown IANA timezone '{name}'")]
    UnknownZone { location: String, name: String },
    #[error("{location}: must be a non-empty list of team ids")]
    TeamIds { location: String },
    #[error("{location}: must hold non-empty text team ids")]
    TeamId { location: String },
    #[error("{location}: entries must be non-empty")]
    EmptyTeamId { location: String },
    #[error("{location}: start and end must differ")]
    SameBounds { location: String },
    #[error("{location}: is not a window field")]
    Extra { location: String },
    #[error("{location}: the current moment is outside the representable range")]
    Moment {
        location: String,
        #[source]
        source: jiff::Error,
    },
}

impl WindowError {
    // pydantic's location of the first failing input: "" for the list itself, "0.start" for one window's field.
    #[must_use]
    pub fn location(&self) -> &str {
        match self {
            Self::NotAList { location }
            | Self::NotAnObject { location }
            | Self::Missing { location }
            | Self::Time { location }
            | Self::Offset { location }
            | Self::ZoneType { location }
            | Self::UnknownZone { location, .. }
            | Self::TeamIds { location }
            | Self::TeamId { location }
            | Self::EmptyTeamId { location }
            | Self::SameBounds { location }
            | Self::Extra { location }
            | Self::Moment { location, .. } => location,
        }
    }
}

// parse_access_windows: an absent or null value has no windows, and every listed window is validated.
pub fn parse_access_windows(
    model_info: &JsonObject,
    database: &TimeZoneDatabase,
) -> Result<Vec<ModelAccessWindow>, WindowError> {
    match model_info.get("access_windows") {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(windows)) => windows
            .iter()
            .enumerate()
            .map(|(index, window)| window_at(index, window, database))
            .collect(),
        Some(_) => Err(WindowError::NotAList {
            location: String::new(),
        }),
    }
}

// access_windows_config_error: the message a malformed value raises while its deployment is built, absent when it parses.
pub fn access_windows_config_error(
    model_info: &JsonObject,
    model_name: &str,
    database: &TimeZoneDatabase,
) -> Option<String> {
    let error = parse_access_windows(model_info, database).err()?;
    Some(format!(
        "model '{model_name}': invalid model_info.access_windows: {error}"
    ))
}

// is_window_active: a window whose start is earlier than its end is a same-day span, and any other is an overnight one.
pub fn is_window_active(
    window: &ModelAccessWindow,
    now: DateTime<Utc>,
) -> Result<bool, WindowError> {
    let timestamp =
        jiff::Timestamp::from_millisecond(now.timestamp_millis()).map_err(|source| {
            WindowError::Moment {
                location: String::new(),
                source,
            }
        })?;
    let local = window.zone.to_datetime(timestamp).time();
    Ok(if window.start < window.end {
        window.start <= local && local < window.end
    } else {
        local >= window.start || local < window.end
    })
}

// The refusal _filter_reserved_deployments raises, which names the end of the window that blocked the pool.
#[must_use]
pub fn reserved_message(model: &str, window: &ModelAccessWindow) -> String {
    format!(
        "Deployment {model} is reserved for another team until {:02}:{:02} {}",
        window.end.hour(),
        window.end.minute(),
        window.timezone
    )
}

fn window_at(
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
    let zone = database
        .get(name)
        .ok()
        .filter(|zone| zone.iana_name() == Some(name.as_str()));
    zone.map_or_else(
        || {
            Err(WindowError::UnknownZone {
                location: location.clone(),
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
            return Err(WindowError::EmptyTeamId {
                location: location.clone(),
            });
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
    clock_shape(text).map_or_else(
        || {
            Err(WindowError::Time {
                location: location.clone(),
            })
        },
        Ok,
    )
}

fn offset_marker(text: &str) -> Option<usize> {
    text.char_indices()
        .skip(1)
        .find(|(_, character)| matches!(character, '+' | '-' | 'Z' | 'z'))
        .map(|(index, _)| index)
}

// HH:MM[:SS[.ffffff]]: the zero-padded shape pydantic's time parser accepts, with at most microsecond fractions.
fn clock_shape(text: &str) -> Option<Time> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 || bytes.get(2) != Some(&b':') {
        return None;
    }
    let hour = digits(bytes.get(..2)?)?;
    let minute = digits(bytes.get(3..5)?)?;
    let (second, nanos) = match bytes.len() {
        5 => (0, 0),
        _ => {
            if bytes.get(5) != Some(&b':') {
                return None;
            }
            let second = digits(bytes.get(6..8)?)?;
            if bytes.len() == 8 {
                (second, 0)
            } else {
                let nanos = fraction(bytes.get(9..).filter(|_| bytes.get(8) == Some(&b'.'))?)?;
                (second, nanos)
            }
        }
    };
    Time::new(
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        nanos,
    )
    .ok()
}

fn digits(bytes: &[u8]) -> Option<i32> {
    if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let tens = i32::from(bytes.first()?.checked_sub(b'0')?);
    let ones = i32::from(bytes.get(1)?.checked_sub(b'0')?);
    tens.checked_mul(10)?.checked_add(ones)
}

// A one-to-six digit fraction, scaled to the nanosecond field jiff's Time keeps.
fn fraction(bytes: &[u8]) -> Option<i32> {
    if bytes.is_empty() || bytes.len() > 6 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let scale = match bytes.len() {
        1 => 100_000_000,
        2 => 10_000_000,
        3 => 1_000_000,
        4 => 100_000,
        5 => 10_000,
        6 => 1_000,
        _ => return None,
    };
    let mut value: i32 = 0;
    for digit in bytes {
        value = value
            .checked_mul(10)?
            .checked_add(i32::from(digit.checked_sub(b'0')?))?;
    }
    value.checked_mul(scale)
}
