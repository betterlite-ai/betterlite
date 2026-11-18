#![cfg(test)]

use rstest::rstest;

use crate::finops::cloudzero::schedule::CloudZeroSettings;

use super::fixtures::{API_KEY, CONNECTION_ID, environment};

#[rstest]
#[case("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "abc")]
#[case("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "1.5")]
#[case("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "9223372036854775808")]
#[case("CLOUDZERO_MAX_FETCHED_DATA_RECORDS", "ten")]
fn cloudzero_schedule(#[case] name: &str, #[case] value: &str) {
    let error = CloudZeroSettings::resolve(
        Some(API_KEY),
        Some(CONNECTION_ID),
        None,
        &environment(&[(name, value)]),
    )
    .expect_err("an invalid number is refused");
    assert_eq!(
        error.to_string(),
        format!("environment variable {name} must be a number")
    );
}
