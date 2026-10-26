use crate::types::WorkerRegistryEntry;

#[test]
fn worker_registry_accepts_http_urls() {
    for url in ["http://worker", "https://worker/path"] {
        let source = format!(r#"{{"worker_id":"id","name":"worker","url":"{url}"}}"#);
        let entry: WorkerRegistryEntry = serde_json::from_str(&source).expect("valid worker URL");
        assert_eq!(entry.url, url);
    }
}

#[test]
fn worker_registry_rejects_non_http_urls() {
    let source = r#"{"worker_id":"id","name":"worker","url":"ftp://worker"}"#;
    let error = serde_json::from_str::<WorkerRegistryEntry>(source).expect_err("invalid worker URL");
    assert!(error.to_string().contains("Worker URL must start with http:// or https://"));
}
