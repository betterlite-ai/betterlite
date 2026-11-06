#![cfg(test)]

use chrono::DateTime;
use rstest::rstest;

use super::{EnvironmentError, to_zod_compatible_iso, validate_environment};
use crate::deepeval::deepeval_logging::types::Environment;

#[test]
fn zod_compatible_iso_writes_utc_milliseconds_with_z() {
    let moment = DateTime::parse_from_rfc3339("2024-05-01T10:00:00.120Z").unwrap();
    assert_eq!(to_zod_compatible_iso(&moment), "2024-05-01T10:00:00.120Z");
}

#[test]
fn zod_compatible_iso_converts_an_offset_to_utc() {
    let moment = DateTime::parse_from_rfc3339("2024-05-01T12:00:00.500+02:00").unwrap();
    assert_eq!(to_zod_compatible_iso(&moment), "2024-05-01T10:00:00.500Z");
}

#[test]
fn zod_compatible_iso_keeps_three_fraction_digits_when_they_are_zero() {
    let moment = DateTime::parse_from_rfc3339("2024-05-01T10:00:00Z").unwrap();
    assert_eq!(to_zod_compatible_iso(&moment), "2024-05-01T10:00:00.000Z");
}

#[rstest]
#[case::production("production", Environment::Production)]
#[case::development("development", Environment::Development)]
#[case::staging("staging", Environment::Staging)]
fn validate_environment_accepts_the_three_names(#[case] name: &str, #[case] expected: Environment) {
    assert_eq!(validate_environment(name).unwrap(), expected);
}

#[test]
fn validate_environment_rejects_another_name() {
    assert_eq!(
        validate_environment("qa").unwrap_err(),
        EnvironmentError::Invalid {
            environment: "qa".to_owned()
        }
    );
}
