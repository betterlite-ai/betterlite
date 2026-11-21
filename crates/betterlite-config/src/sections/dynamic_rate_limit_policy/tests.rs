#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::Section;

use super::{
    PriorityReservationSetting, Raw, RawLitellmSettings, RawPrioritySettings, RawPriorityValue,
    ReservationKind, Settings,
};

fn settings(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the config should load")
    .sections
    .dynamic_rate_limit_policy
}

fn load_error(source: &str) -> ConfigError {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect_err("the config should be rejected")
}

fn raw(
    settings: RawPrioritySettings,
    reservation: Option<BTreeMap<String, RawPriorityValue>>,
) -> Raw {
    Raw {
        litellm_settings: RawLitellmSettings {
            priority_reservation: reservation,
            priority_reservation_settings: Some(settings),
        },
    }
}

fn convert(raw: Raw) -> Result<Settings, ConfigError> {
    raw.convert(&BTreeMap::<String, String>::new())
}

#[test]
fn absent_section_reads_the_litellm_defaults() {
    let settings = settings("litellm_settings: {}\n");
    assert_eq!(settings.default_priority.to_bits(), 0.25_f64.to_bits());
    assert_eq!(settings.saturation_threshold.to_bits(), 0.50_f64.to_bits());
    assert_eq!(settings.saturation_check_cache_ttl, 60);
    assert_eq!(settings.priority_reservation, None);
}

#[test]
fn explicit_settings_are_read() {
    let settings = settings(
        "litellm_settings:\n  priority_reservation_settings:\n    default_priority: 0.1\n    saturation_threshold: 0.75\n    saturation_check_cache_ttl: 5\n",
    );
    assert_eq!(settings.default_priority.to_bits(), 0.1_f64.to_bits());
    assert_eq!(settings.saturation_threshold.to_bits(), 0.75_f64.to_bits());
    assert_eq!(settings.saturation_check_cache_ttl, 5);
}

#[rstest]
#[case::bare_percent("key_a: 0.6", ReservationKind::Percent, 0.6)]
#[case::percent_mapping("key_a: {type: percent, value: 0.4}", ReservationKind::Percent, 0.4)]
#[case::rpm("key_a: {type: rpm, value: 900}", ReservationKind::Rpm, 900.0)]
#[case::tpm("key_a: {type: tpm, value: 9000}", ReservationKind::Tpm, 9000.0)]
#[case::unknown_type("key_a: {type: banana, value: 0.2}", ReservationKind::Percent, 0.2)]
#[case::absent_type("key_a: {value: 0.3}", ReservationKind::Percent, 0.3)]
#[case::absent_value("key_a: {type: rpm}", ReservationKind::Rpm, 1.0)]
fn reservation_forms(#[case] entry: &str, #[case] kind: ReservationKind, #[case] value: f64) {
    let source = format!("litellm_settings:\n  priority_reservation:\n    {entry}\n");
    let settings = settings(&source);
    assert_eq!(
        settings.priority_reservation,
        Some(BTreeMap::from([(
            "key_a".to_owned(),
            PriorityReservationSetting { kind, value }
        )]))
    );
}

#[rstest]
#[case::default_priority(
    RawPrioritySettings {
        default_priority: Some(f64::NAN),
        saturation_threshold: None,
        saturation_check_cache_ttl: None,
    },
    "litellm_settings.priority_reservation_settings.default_priority"
)]
#[case::saturation_threshold(
    RawPrioritySettings {
        default_priority: None,
        saturation_threshold: Some(f64::INFINITY),
        saturation_check_cache_ttl: None,
    },
    "litellm_settings.priority_reservation_settings.saturation_threshold"
)]
fn non_finite_settings_are_rejected(#[case] settings: RawPrioritySettings, #[case] path: &str) {
    let error = convert(raw(settings, None)).expect_err("a non-finite setting should be rejected");
    assert!(
        matches!(error, ConfigError::InvalidScalar { path: reported, .. } if reported == path),
        "unexpected error: {error}"
    );
}

#[test]
fn a_non_finite_reservation_value_is_rejected() {
    let reservation = BTreeMap::from([(
        "key_a".to_owned(),
        RawPriorityValue::Percent(f64::NEG_INFINITY),
    )]);
    let error = convert(raw(RawPrioritySettings::default(), Some(reservation)))
        .expect_err("a non-finite reservation should be rejected");
    assert!(
        matches!(error, ConfigError::InvalidScalar { path, .. } if path == "litellm_settings.priority_reservation.key_a"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_text_reservation_is_rejected() {
    let error = load_error("litellm_settings:\n  priority_reservation:\n    key_a: \"0.6\"\n");
    assert!(
        matches!(error, ConfigError::Yaml { .. }),
        "unexpected error: {error}"
    );
}
