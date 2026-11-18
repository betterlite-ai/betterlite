#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::finops::cloudzero::schedule::CloudZeroSettings;

use super::fixtures::{API_KEY, CONNECTION_ID, environment};

type Expectation = (&'static str, &'static str, &'static str, i64, i64);

#[rstest]
#[case::defaults(None, None, None, &[], ("", "", "UTC", 60, 50_000))]
#[case::parameters(
    Some("p-key"),
    Some("p-conn"),
    Some("Europe/Berlin"),
    &[],
    ("p-key", "p-conn", "Europe/Berlin", 60, 50_000)
)]
#[case::environment(
    None,
    None,
    None,
    &[
        ("CLOUDZERO_API_KEY", "e-key"),
        ("CLOUDZERO_CONNECTION_ID", "e-conn"),
        ("CLOUDZERO_TIMEZONE", "Asia/Tokyo"),
    ],
    ("e-key", "e-conn", "Asia/Tokyo", 60, 50_000)
)]
#[case::parameters_win(
    Some("p-key"),
    Some("p-conn"),
    Some("p-tz"),
    &[
        ("CLOUDZERO_API_KEY", "e-key"),
        ("CLOUDZERO_CONNECTION_ID", "e-conn"),
        ("CLOUDZERO_TIMEZONE", "e-tz"),
    ],
    ("p-key", "p-conn", "p-tz", 60, 50_000)
)]
#[case::empty_parameters_fall_back(
    Some(""),
    Some(""),
    Some(""),
    &[
        ("CLOUDZERO_API_KEY", "e-key"),
        ("CLOUDZERO_CONNECTION_ID", "e-conn"),
        ("CLOUDZERO_TIMEZONE", "e-tz"),
    ],
    ("e-key", "e-conn", "e-tz", 60, 50_000)
)]
#[case::empty_environment_is_unset(
    None,
    None,
    None,
    &[
        ("CLOUDZERO_API_KEY", ""),
        ("CLOUDZERO_CONNECTION_ID", ""),
        ("CLOUDZERO_TIMEZONE", ""),
    ],
    ("", "", "UTC", 60, 50_000)
)]
#[case::numbers_from_environment(
    None,
    None,
    None,
    &[
        ("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "30"),
        ("CLOUDZERO_MAX_FETCHED_DATA_RECORDS", "7"),
    ],
    ("", "", "UTC", 30, 7)
)]
#[case::explicit_zero_is_kept(
    None,
    None,
    None,
    &[
        ("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "0"),
        ("CLOUDZERO_MAX_FETCHED_DATA_RECORDS", "0"),
    ],
    ("", "", "UTC", 0, 0)
)]
#[case::negative_interval_is_kept(
    None,
    None,
    None,
    &[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "-5")],
    ("", "", "UTC", -5, 50_000)
)]
fn cloudzero_schedule(
    #[case] api_key: Option<&str>,
    #[case] connection_id: Option<&str>,
    #[case] timezone: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Expectation,
) {
    let settings =
        CloudZeroSettings::resolve(api_key, connection_id, timezone, &environment(pairs))
            .expect("resolved settings");
    let expected_key = (!expected.0.is_empty()).then_some(expected.0);
    let expected_connection = (!expected.1.is_empty()).then_some(expected.1);
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(
        settings
            .connection_id
            .as_ref()
            .map(ExposeSecret::expose_secret),
        expected_connection
    );
    assert_eq!(settings.timezone, expected.2);
    assert_eq!(settings.interval_minutes, expected.3);
    assert_eq!(settings.max_fetched_records, expected.4);
}
