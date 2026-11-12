#![cfg(test)]

use secrecy::{ExposeSecret as _, SecretString};

use crate::langfuse_otel::destination::build_langfuse_otel_headers;

#[test]
fn header_construction_holds_authorization_then_the_ingestion_version() {
    let headers = build_langfuse_otel_headers(SecretString::from("Basic abc"));
    let entries: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.expose_secret()))
        .collect();
    assert_eq!(
        entries,
        vec![
            ("Authorization", "Basic abc"),
            ("x-langfuse-ingestion-version", "4"),
        ]
    );
}
