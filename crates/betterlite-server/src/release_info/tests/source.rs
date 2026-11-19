use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::release_info::records::FIXED_RELEASE_URL;
use crate::release_info::{RELEASE_FIXTURE_VARIABLE, release_source_for, system_seconds};

type Environment = BTreeMap<String, Option<SecretString>>;

fn configured(source: &str) -> Environment {
    let mut environment = Environment::new();
    environment.insert(
        RELEASE_FIXTURE_VARIABLE.to_owned(),
        Some(SecretString::from(source.to_owned())),
    );
    environment
}

#[test]
fn the_fixed_url_is_used_without_a_configured_destination() {
    let environment = Environment::new();
    assert_eq!(
        release_source_for(&environment, true),
        FIXED_RELEASE_URL,
        "the fixture falls back to the fixed release url"
    );
}

#[test]
fn a_configured_destination_replaces_the_fixed_url() {
    let environment = configured("http://127.0.0.1:9/releases/latest");
    assert_eq!(
        release_source_for(&environment, true),
        "http://127.0.0.1:9/releases/latest"
    );
}

#[test]
fn production_ignores_the_configured_destination() {
    let environment = configured("http://127.0.0.1:9/releases/latest");
    assert_eq!(
        release_source_for(&environment, false),
        FIXED_RELEASE_URL,
        "a production build never reads the fixture destination"
    );
}

#[test]
fn a_destination_without_a_value_keeps_the_fixed_url() {
    let mut environment = Environment::new();
    environment.insert(RELEASE_FIXTURE_VARIABLE.to_owned(), None);
    assert_eq!(release_source_for(&environment, true), FIXED_RELEASE_URL);
}

#[test]
fn the_clock_reads_the_current_time() {
    assert!(system_seconds() > 1_600_000_000.0_f64);
}
