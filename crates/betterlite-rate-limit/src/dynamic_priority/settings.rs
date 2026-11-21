use std::collections::BTreeMap;

use betterlite_config::sections::dynamic_rate_limit_policy as config_policy;

use crate::provider_attribution::{PriorityKind, PriorityReservation, PriorityValue};

mod tests;

// PriorityReservationSettings: the three numbers every hook reads off the loaded section
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrioritySettings {
    pub default_priority: f64,
    pub saturation_threshold: f64,
    pub saturation_check_cache_ttl: i64,
}

impl Default for PrioritySettings {
    fn default() -> Self {
        Self::from(&config_policy::Settings::default())
    }
}

impl From<&config_policy::Settings> for PrioritySettings {
    fn from(configured: &config_policy::Settings) -> Self {
        Self {
            default_priority: configured.default_priority,
            saturation_threshold: configured.saturation_threshold,
            saturation_check_cache_ttl: configured.saturation_check_cache_ttl,
        }
    }
}

// litellm.priority_reservation next to the settings that read it, resolved once when the proxy starts
#[derive(Clone, Debug, PartialEq)]
pub struct PriorityPolicy {
    pub settings: PrioritySettings,
    pub reservation: Option<BTreeMap<String, PriorityValue>>,
}

impl Default for PriorityPolicy {
    fn default() -> Self {
        Self::from_config(&config_policy::Settings::default())
    }
}

impl PriorityPolicy {
    // the section's resolved values, with each reservation keeping the kind its config entry named
    #[must_use]
    pub fn from_config(configured: &config_policy::Settings) -> Self {
        Self {
            settings: PrioritySettings::from(configured),
            reservation: configured
                .priority_reservation
                .as_ref()
                .map(reservation_values),
        }
    }
}

// _get_priority_settings: a hook that reads the reservation settings never sees them absent, and an unloaded policy reads the defaults
#[must_use]
pub fn get_priority_settings(configured: Option<&config_policy::Settings>) -> PrioritySettings {
    let Some(section) = configured else {
        return PrioritySettings::default();
    };
    PrioritySettings::from(section)
}

fn reservation_values(
    configured: &BTreeMap<String, config_policy::PriorityReservationSetting>,
) -> BTreeMap<String, PriorityValue> {
    let mut values = BTreeMap::new();
    for (priority, setting) in configured {
        values.insert(priority.clone(), priority_value(*setting));
    }
    values
}

const fn priority_value(setting: config_policy::PriorityReservationSetting) -> PriorityValue {
    match setting.kind {
        config_policy::ReservationKind::Percent => PriorityValue::Percent(setting.value),
        config_policy::ReservationKind::Rpm => PriorityValue::Reservation(PriorityReservation {
            kind: PriorityKind::Rpm,
            value: setting.value,
        }),
        config_policy::ReservationKind::Tpm => PriorityValue::Reservation(PriorityReservation {
            kind: PriorityKind::Tpm,
            value: setting.value,
        }),
    }
}
