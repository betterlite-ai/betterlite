#![cfg(test)]

use super::TlsMode;
use crate::connection::{FixtureSource, select_source};
use secrecy::SecretString;
use tokio_postgres::config::SslMode;

#[test]
fn missing_sslmode_selects_prefer() {
    let configured = SecretString::from("postgresql://u:p@localhost/db");
    let FixtureSource::Configured(options) = select_source(Some(&configured)).unwrap() else {
        panic!("configured source")
    };
    assert_eq!(options.tls_mode, TlsMode::Prefer);
    assert_eq!(options.config.get_ssl_mode(), SslMode::Prefer);
}
