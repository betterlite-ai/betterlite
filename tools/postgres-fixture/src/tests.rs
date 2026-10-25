#![cfg(test)]

use crate::connection::{FixtureSource, select_source};
use crate::error::FixtureError;

#[test]
fn configured_url_wins() {
    assert!(matches!(
        select_source(Some("postgresql://u:p@localhost/db")),
        Ok(FixtureSource::Configured(_))
    ));
}

#[test]
fn containers_when_unset() {
    assert!(matches!(
        select_source(None),
        Ok(FixtureSource::Testcontainers)
    ));
}

#[test]
fn empty_or_malformed_urls_and_credentials_are_handled() {
    assert!(matches!(
        select_source(Some("")),
        Err(FixtureError::EmptyConfiguredUrl)
    ));
    assert!(matches!(
        select_source(Some("not-postgres")),
        Err(FixtureError::InvalidConfiguredUrl)
    ));
    let FixtureSource::Configured(options) =
        select_source(Some("postgresql://u:secret@localhost/db")).unwrap()
    else {
        panic!("configured source")
    };
    assert!(!format!("{options:?}").contains("secret"));
}
