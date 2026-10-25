#![cfg(test)]

use super::TlsMode;
use crate::connection::{FixtureSource, select_source};
use crate::error::FixtureError;
use secrecy::SecretString;
use tokio_postgres::config::SslMode;

#[test]
fn missing_sslmode_disables_tls() {
    let configured = SecretString::from("postgresql://u:p@localhost/db");
    let FixtureSource::Configured(options) = select_source(Some(&configured)).unwrap() else {
        panic!("configured source")
    };
    assert_eq!(options.tls_mode, TlsMode::Disable);
    assert_eq!(options.config.get_ssl_mode(), SslMode::Disable);
}

#[test]
fn explicit_sslmode_prefer_is_rejected() {
    let configured = SecretString::from("postgresql://u:p@localhost/db?sslmode=prefer");
    assert!(matches!(
        select_source(Some(&configured)),
        Err(FixtureError::ExplicitPreferTls)
    ));
}
