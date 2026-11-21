#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use super::{MAX_ATTEMPTS_ENV, RETRY_INTERVAL_ENV, SessionLookupSettings};

const fn lookup(max_attempts: u64, retry_interval_seconds: f64) -> SessionLookupSettings {
    SessionLookupSettings {
        max_attempts,
        retry_interval_seconds,
    }
}

fn invalid(name: &str) -> Result<SessionLookupSettings, EnvError> {
    Err(EnvError::Number {
        name: name.to_owned(),
    })
}

#[rstest]
#[case::unset(None, None, Ok(lookup(3_u64, 0.2_f64)))]
#[case::blank(Some(""), Some(" "), Ok(lookup(3_u64, 0.2_f64)))]
#[case::zero_attempts(Some("0"), None, Ok(lookup(1_u64, 0.2_f64)))]
#[case::negative_attempts(Some("-2"), None, Ok(lookup(1_u64, 0.2_f64)))]
#[case::one_attempt(Some("1"), None, Ok(lookup(1_u64, 0.2_f64)))]
#[case::two_attempts(Some("2"), None, Ok(lookup(2_u64, 0.2_f64)))]
#[case::five_attempts(Some("5"), None, Ok(lookup(5_u64, 0.2_f64)))]
#[case::zero_interval(None, Some("0"), Ok(lookup(3_u64, 0.0_f64)))]
#[case::negative_interval(None, Some("-1"), Ok(lookup(3_u64, -1.0_f64)))]
#[case::padded_interval(None, Some(" 1.5 "), Ok(lookup(3_u64, 1.5_f64)))]
#[case::bad_attempts(Some("bad"), None, invalid(MAX_ATTEMPTS_ENV))]
#[case::fractional_attempts(Some("2.5"), None, invalid(MAX_ATTEMPTS_ENV))]
#[case::bad_interval(None, Some("bad"), invalid(RETRY_INTERVAL_ENV))]
fn loads_session_lookup_settings(
    #[case] attempts: Option<&str>,
    #[case] interval: Option<&str>,
    #[case] expected: Result<SessionLookupSettings, EnvError>,
) {
    let mut environment = BTreeMap::new();
    if let Some(text) = attempts {
        environment.insert(MAX_ATTEMPTS_ENV.to_owned(), text.to_owned());
    }
    if let Some(text) = interval {
        environment.insert(RETRY_INTERVAL_ENV.to_owned(), text.to_owned());
    }
    assert_eq!(super::loads_session_lookup_settings(&environment), expected);
}
