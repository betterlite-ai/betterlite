use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use jiff::civil::Time;
use jiff::tz::{TimeZone, TimeZoneDatabase};
use thiserror::Error;

use self::decode::window_at;

mod decode;

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
