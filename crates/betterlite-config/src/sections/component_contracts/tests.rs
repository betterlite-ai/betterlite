#![cfg(test)]

use super::WorkerRegistryEntry;

#[test]
fn worker_registry_accepts_http_urls() {
    let http = WorkerRegistryEntry::validate_url("http://worker");
    let https = WorkerRegistryEntry::validate_url("https://worker/path");
    http.expect("http URL should validate");
    https.expect("https URL should validate");
}

#[test]
fn worker_registry_rejects_non_http_urls() {
    let error = WorkerRegistryEntry::validate_url("ftp://worker").expect_err("invalid URL");
    assert_eq!(
        error.to_string(),
        "Worker URL must start with http:// or https://"
    );
}

#[test]
fn worker_registry_deserialization_rejects_non_http_urls() {
    let error = serde_json::from_str::<WorkerRegistryEntry>(
        r#"{"worker_id":"worker-1","name":"worker","url":"ftp://worker"}"#,
    )
    .expect_err("invalid URL should fail during deserialization");
    assert!(error.to_string().contains("Worker URL must start with http:// or https://"));
}
