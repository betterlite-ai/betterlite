#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::dynamic_rate_limit_policy as config_policy;

use super::{PriorityPolicy, PrioritySettings, get_priority_settings};
use crate::dynamic_priority::tests::support::{percent, reservation_map, reserved};
use crate::provider_attribution::PriorityKind;

fn section(
    default_priority: f64,
    saturation_threshold: f64,
    saturation_check_cache_ttl: i64,
    priority_reservation: Option<BTreeMap<String, config_policy::PriorityReservationSetting>>,
) -> config_policy::Settings {
    config_policy::Settings {
        default_priority,
        saturation_threshold,
        saturation_check_cache_ttl,
        priority_reservation,
    }
}

fn reservation(
    kind: config_policy::ReservationKind,
    value: f64,
) -> config_policy::PriorityReservationSetting {
    config_policy::PriorityReservationSetting { kind, value }
}

#[test]
fn the_defaults_are_litellms() {
    let defaults = PrioritySettings::default();
    assert_eq!(defaults.default_priority.to_bits(), 0.25_f64.to_bits());
    assert_eq!(defaults.saturation_threshold.to_bits(), 0.50_f64.to_bits());
    assert_eq!(defaults.saturation_check_cache_ttl, 60);
    assert_eq!(PriorityPolicy::default().settings, defaults);
    assert_eq!(PriorityPolicy::default().reservation, None);
}

#[test]
fn an_unloaded_settings_section_reads_the_defaults() {
    assert_eq!(get_priority_settings(None), PrioritySettings::default());
}

#[test]
fn a_loaded_settings_section_is_read_as_it_is() {
    let configured = section(0.1, 0.75, 5, None);
    let settings = get_priority_settings(Some(&configured));
    assert_eq!(settings.default_priority.to_bits(), 0.1_f64.to_bits());
    assert_eq!(settings.saturation_threshold.to_bits(), 0.75_f64.to_bits());
    assert_eq!(settings.saturation_check_cache_ttl, 5);
}

#[test]
fn from_config_maps_every_reservation_kind() {
    let reservations = BTreeMap::from([
        (
            "prod".to_owned(),
            reservation(config_policy::ReservationKind::Percent, 0.5),
        ),
        (
            "gold".to_owned(),
            reservation(config_policy::ReservationKind::Rpm, 600.0),
        ),
        (
            "silver".to_owned(),
            reservation(config_policy::ReservationKind::Tpm, 900.0),
        ),
    ]);
    let policy = PriorityPolicy::from_config(&section(0.1, 0.75, 120, Some(reservations)));
    assert_eq!(
        policy.reservation,
        Some(reservation_map(&[
            ("prod", percent(0.5)),
            ("gold", reserved(PriorityKind::Rpm, 600.0)),
            ("silver", reserved(PriorityKind::Tpm, 900.0)),
        ]))
    );
    assert_eq!(
        policy.settings,
        PrioritySettings {
            default_priority: 0.1,
            saturation_threshold: 0.75,
            saturation_check_cache_ttl: 120,
        }
    );
}

#[test]
fn an_empty_reservation_mapping_stays_present() {
    let policy = PriorityPolicy::from_config(&section(0.25, 0.50, 60, Some(BTreeMap::new())));
    assert_eq!(policy.reservation, Some(BTreeMap::new()));
}
