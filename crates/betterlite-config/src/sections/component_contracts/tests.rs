#![cfg(test)]

use super::WorkerRegistryEntry;

#[test]
fn worker_registry_accepts_http_urls() {
    let http = WorkerRegistryEntry::validate_url("http://worker");
    let https = WorkerRegistryEntry::validate_url("https://worker/path");
    assert!(http.is_ok());
    assert!(https.is_ok());
}

#[test]
fn worker_registry_rejects_non_http_urls() {
    let error = WorkerRegistryEntry::validate_url("ftp://worker").expect_err("invalid URL");
    assert_eq!(
        error.to_string(),
        "Worker URL must start with http:// or https://"
    );
}
