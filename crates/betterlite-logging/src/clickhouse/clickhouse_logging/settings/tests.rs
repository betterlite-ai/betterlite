#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{
    BATCH_SIZE_VARIABLE, ClickHouseSettings, ClickHouseSettingsError, DATABASE_VARIABLE,
    DEFAULT_BATCH_SIZE, DEFAULT_DATABASE, DEFAULT_MAX_BUFFERED_ROWS, DEFAULT_MAX_RETRIES,
    FLUSH_INTERVAL_VARIABLE, MAX_BUFFERED_ROWS_VARIABLE, MAX_RETRIES_VARIABLE, URL_VARIABLE,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn an_empty_environment_takes_every_clickhouse_default() {
    let settings =
        ClickHouseSettings::from_environment(&BTreeMap::new()).expect("the defaults load");
    assert_eq!(settings.database, DEFAULT_DATABASE);
    assert_eq!(settings.url.expose_secret(), "");
    assert_eq!(settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(settings.flush_interval, Duration::from_secs(1));
    assert_eq!(settings.max_buffered_rows, DEFAULT_MAX_BUFFERED_ROWS);
    assert_eq!(settings.max_retries, DEFAULT_MAX_RETRIES);
}

#[test]
fn reads_every_clickhouse_variable_from_the_environment() {
    let settings = ClickHouseSettings::from_environment(&environment(&[
        (DATABASE_VARIABLE, "traces"),
        (URL_VARIABLE, "http://default:secret@clickhouse:8123"),
        (BATCH_SIZE_VARIABLE, "25"),
        (FLUSH_INTERVAL_VARIABLE, "0.25"),
        (MAX_BUFFERED_ROWS_VARIABLE, "40"),
        (MAX_RETRIES_VARIABLE, "6"),
    ]))
    .expect("the values load");
    assert_eq!(settings.database, "traces");
    assert_eq!(
        settings.url.expose_secret(),
        "http://default:secret@clickhouse:8123"
    );
    assert_eq!(settings.batch_size, 25);
    assert_eq!(settings.flush_interval, Duration::from_millis(250));
    assert_eq!(settings.max_buffered_rows, 40);
    assert_eq!(settings.max_retries, 6);
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let settings = ClickHouseSettings::from_environment(&environment(&[
        (DATABASE_VARIABLE, ""),
        (URL_VARIABLE, ""),
        (BATCH_SIZE_VARIABLE, ""),
        (FLUSH_INTERVAL_VARIABLE, ""),
        (MAX_BUFFERED_ROWS_VARIABLE, ""),
        (MAX_RETRIES_VARIABLE, ""),
    ]))
    .expect("empty values are unset");
    assert_eq!(settings.database, DEFAULT_DATABASE);
    assert_eq!(settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(settings.flush_interval, Duration::from_secs(1));
    assert_eq!(settings.max_buffered_rows, DEFAULT_MAX_BUFFERED_ROWS);
    assert_eq!(settings.max_retries, DEFAULT_MAX_RETRIES);
}

#[rstest]
#[case::text(BATCH_SIZE_VARIABLE, "many")]
#[case::negative(BATCH_SIZE_VARIABLE, "-1")]
#[case::float(BATCH_SIZE_VARIABLE, "1.5")]
#[case::exponent(MAX_RETRIES_VARIABLE, "1e3")]
#[case::text_rows(MAX_BUFFERED_ROWS_VARIABLE, "lots")]
fn rejects_a_clickhouse_count_that_is_not_an_integer(
    #[case] name: &'static str,
    #[case] value: &str,
) {
    let error = ClickHouseSettings::from_environment(&environment(&[(name, value)]))
        .expect_err("the value is not an integer");
    assert!(matches!(error, ClickHouseSettingsError::Environment(_)));
}

#[test]
fn rejects_a_zero_batch_size() {
    let error = ClickHouseSettings::from_environment(&environment(&[(BATCH_SIZE_VARIABLE, "0")]))
        .expect_err("a zero batch size would flush forever");
    assert!(matches!(
        error,
        ClickHouseSettingsError::Zero {
            name: BATCH_SIZE_VARIABLE
        }
    ));
}

#[rstest]
#[case::negative("-1.5")]
#[case::text("soon")]
#[case::exponent_times_seconds("1e400")]
fn rejects_a_flush_interval_that_is_not_a_duration(#[case] value: &str) {
    let error =
        ClickHouseSettings::from_environment(&environment(&[(FLUSH_INTERVAL_VARIABLE, value)]))
            .expect_err("the interval is not a duration");
    assert!(matches!(
        error,
        ClickHouseSettingsError::Environment(_) | ClickHouseSettingsError::Interval { .. }
    ));
}
