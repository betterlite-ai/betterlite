use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const DEFAULT_PRIORITY: f64 = 0.25;
const SATURATION_THRESHOLD: f64 = 0.50;
const SATURATION_CHECK_CACHE_TTL: i64 = 60;
const DEFAULT_PRIORITY_PATH: &str =
    "litellm_settings.priority_reservation_settings.default_priority";
const SATURATION_THRESHOLD_PATH: &str =
    "litellm_settings.priority_reservation_settings.saturation_threshold";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    priority_reservation: Option<BTreeMap<String, RawPriorityValue>>,
    priority_reservation_settings: Option<RawPrioritySettings>,
}

// PriorityReservationDict: a bare number is already a share of capacity, and a mapping says what its value counts
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPriorityValue {
    Percent(f64),
    Reservation(RawPriorityReservation),
}

#[derive(Debug, Deserialize)]
struct RawPriorityReservation {
    #[serde(rename = "type")]
    kind: Option<String>,
    value: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPrioritySettings {
    default_priority: Option<f64>,
    saturation_threshold: Option<f64>,
    saturation_check_cache_ttl: Option<i64>,
}

// PriorityReservationDict's "type": what a reservation's value counts against the model group's capacity
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationKind {
    Percent,
    Rpm,
    Tpm,
}

impl ReservationKind {
    // convert_priority_to_percent reads every name but rpm and tpm as a percent, and so does an absent type
    fn from_wire(kind: Option<&str>) -> Self {
        match kind {
            Some("rpm") => Self::Rpm,
            Some("tpm") => Self::Tpm,
            Some(_) | None => Self::Percent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriorityReservationSetting {
    pub kind: ReservationKind,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub default_priority: f64,
    pub saturation_threshold: f64,
    pub saturation_check_cache_ttl: i64,
    pub priority_reservation: Option<BTreeMap<String, PriorityReservationSetting>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_priority: DEFAULT_PRIORITY,
            saturation_threshold: SATURATION_THRESHOLD,
            saturation_check_cache_ttl: SATURATION_CHECK_CACHE_TTL,
            priority_reservation: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = self
            .litellm_settings
            .priority_reservation_settings
            .unwrap_or_default();
        Ok(Settings {
            default_priority: finite_or(
                configured.default_priority.unwrap_or(DEFAULT_PRIORITY),
                DEFAULT_PRIORITY_PATH,
            )?,
            saturation_threshold: finite_or(
                configured
                    .saturation_threshold
                    .unwrap_or(SATURATION_THRESHOLD),
                SATURATION_THRESHOLD_PATH,
            )?,
            saturation_check_cache_ttl: configured
                .saturation_check_cache_ttl
                .unwrap_or(SATURATION_CHECK_CACHE_TTL),
            priority_reservation: self
                .litellm_settings
                .priority_reservation
                .map(reservation_settings)
                .transpose()?,
        })
    }
}

fn reservation_settings(
    configured: BTreeMap<String, RawPriorityValue>,
) -> Result<BTreeMap<String, PriorityReservationSetting>, ConfigError> {
    let mut settings = BTreeMap::new();
    for (priority, value) in configured {
        let setting = reservation_setting(value, &priority)?;
        settings.insert(priority, setting);
    }
    Ok(settings)
}

fn reservation_setting(
    value: RawPriorityValue,
    priority: &str,
) -> Result<PriorityReservationSetting, ConfigError> {
    let (kind, number) = match value {
        RawPriorityValue::Percent(percent) => (ReservationKind::Percent, percent),
        RawPriorityValue::Reservation(reservation) => (
            ReservationKind::from_wire(reservation.kind.as_deref()),
            reservation.value.unwrap_or(1.0_f64),
        ),
    };
    let path = format!("litellm_settings.priority_reservation.{priority}");
    Ok(PriorityReservationSetting {
        kind,
        value: finite_or(number, &path)?,
    })
}

// pydantic stores a NaN or an infinity as it is, and the first int() of it fails every request, so the load stops here
fn finite_or(value: f64, path: &str) -> Result<f64, ConfigError> {
    if value.is_finite() {
        return Ok(value);
    }
    Err(ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    })
}
