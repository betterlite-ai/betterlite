#![cfg(test)]

use std::collections::BTreeMap;
use std::mem::size_of;

use crate::sections::Section as _;
use crate::sections::focus_export_schedule::{Inner, Raw, Settings};

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn an_empty_environment_gives_the_documented_defaults() {
    let settings = Settings::from_environment(&BTreeMap::new()).unwrap();
    let documented = Settings {
        inner: Box::new(Inner {
            provider: "s3".to_owned(),
            export_format: "parquet".to_owned(),
            frequency: "hourly".to_owned(),
            cron_offset_minute: 5,
            interval_seconds: None,
            prefix: "focus_exports".to_owned(),
        }),
    };
    assert_eq!(settings, documented);
    assert_eq!(Settings::default(), documented);
}

#[test]
fn the_environment_supplies_every_field_in_lowercase() {
    let settings = Settings::from_environment(&environment(&[
        ("FOCUS_PROVIDER", "VANTAGE"),
        ("FOCUS_FORMAT", "CSV"),
        ("FOCUS_FREQUENCY", "Daily"),
        ("FOCUS_CRON_OFFSET", "17"),
        ("FOCUS_INTERVAL_SECONDS", "120"),
        ("FOCUS_PREFIX", "Exports"),
    ]))
    .unwrap();
    assert_eq!(settings.inner.provider, "vantage");
    assert_eq!(settings.inner.export_format, "csv");
    assert_eq!(settings.inner.frequency, "daily");
    assert_eq!(settings.inner.cron_offset_minute, 17);
    assert_eq!(settings.inner.interval_seconds, Some(120));
    assert_eq!(settings.inner.prefix, "Exports");
}

#[test]
fn empty_variables_count_as_unset() {
    let settings = Settings::from_environment(&environment(&[
        ("FOCUS_PROVIDER", ""),
        ("FOCUS_FORMAT", ""),
        ("FOCUS_FREQUENCY", ""),
        ("FOCUS_CRON_OFFSET", ""),
        ("FOCUS_INTERVAL_SECONDS", ""),
        ("FOCUS_PREFIX", ""),
    ]))
    .unwrap();
    assert_eq!(settings, Settings::default());
}

#[test]
fn an_invalid_cron_offset_names_the_variable() {
    let error =
        Settings::from_environment(&environment(&[("FOCUS_CRON_OFFSET", "soon")])).unwrap_err();
    assert_eq!(
        error.to_string(),
        "environment variable FOCUS_CRON_OFFSET must be a number"
    );
}

#[test]
fn an_invalid_interval_reads_as_unset() {
    let settings =
        Settings::from_environment(&environment(&[("FOCUS_INTERVAL_SECONDS", "half")])).unwrap();
    assert_eq!(settings.inner.interval_seconds, None);
}

#[test]
fn the_settings_stay_behind_one_pointer() {
    assert_eq!(size_of::<Settings>(), size_of::<usize>());
}

#[test]
fn the_section_converts_the_environment() {
    let raw: Raw = serde_saphyr::from_str("{}").unwrap();
    let settings = raw
        .convert(&environment(&[("FOCUS_PREFIX", "chosen")]))
        .unwrap();
    assert_eq!(settings.inner.prefix, "chosen");
}
