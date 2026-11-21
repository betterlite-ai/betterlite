#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::Level;
use tracing::subscriber::with_default;

use crate::loader::{ConfigError, ConfigLoader};

use super::{DEFAULT_TIMEOUT, Settings, TIMEOUT_VARIABLE};

fn settings(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .map(|config| config.sections.prometheus_request_budget)
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

#[rstest]
#[case::unset(None, DEFAULT_TIMEOUT)]
#[case::empty(Some(""), DEFAULT_TIMEOUT)]
#[case::fractional(Some("2.5"), Duration::from_millis(2_500))]
#[case::zero(Some("0"), DEFAULT_TIMEOUT)]
#[case::negative(Some("-3"), DEFAULT_TIMEOUT)]
#[case::overflowing_float(Some("1e999"), DEFAULT_TIMEOUT)]
#[case::unrepresentable_duration(Some("1e300"), Duration::MAX)]
fn resolves_the_per_request_timeout(#[case] text: Option<&str>, #[case] expected: Duration) {
    let variables: Vec<(&str, &str)> = text
        .map(|text| (TIMEOUT_VARIABLE, text))
        .into_iter()
        .collect();
    let loaded = settings(&variables).expect("the timeout loads");
    assert_eq!(loaded.per_request_timeout, expected);
}

#[test]
fn refuses_text_that_is_not_a_number() {
    let error = settings(&[(TIMEOUT_VARIABLE, "abc")]).expect_err("a word is not a number");
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: betterlite_env::EnvError::Number { name },
        } if name == TIMEOUT_VARIABLE
    ));
}

#[test]
fn logs_the_invalid_value_fallback() {
    let (loaded, log) = logged(|| settings(&[(TIMEOUT_VARIABLE, "0")]));
    assert_eq!(
        loaded.expect("the fallback loads").per_request_timeout,
        DEFAULT_TIMEOUT
    );
    assert!(
        log.contains(
            "invalid PROMETHEUS_BUDGET_METRICS_PER_REQUEST_TIMEOUT=0; using default 5.0s."
        ),
        "{log}"
    );
}

#[test]
fn stays_quiet_for_a_usable_value() {
    let (loaded, log) = logged(|| settings(&[(TIMEOUT_VARIABLE, "2.5")]));
    assert_eq!(
        loaded.expect("the timeout loads").per_request_timeout,
        Duration::from_millis(2_500)
    );
    assert!(!log.contains("invalid"), "{log}");
}
