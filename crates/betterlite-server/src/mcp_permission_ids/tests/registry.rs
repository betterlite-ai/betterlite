use crate::mcp_permission_ids::registry::allow_all_keys_server_ids;

use super::fixtures::{registry, registry_server, string_set};

#[tokio::test]
async fn allow_all_keys_snapshot() {
    let config = vec![
        registry_server(
            r#"{"server_id": "cfg-flag", "name": "cfg-flag", "transport": "http", "allow_all_keys": true}"#,
        ),
        registry_server(
            r#"{"server_id": "cfg-shared", "name": "cfg-shared", "transport": "http", "allow_all_keys": true}"#,
        ),
        registry_server(r#"{"server_id": "cfg-plain", "name": "cfg-plain", "transport": "http"}"#),
    ];
    let db = vec![
        registry_server(r#"{"server_id": "cfg-shared", "name": "db-shared", "transport": "http"}"#),
        registry_server(
            r#"{"server_id": "db-flag", "name": "db-flag", "transport": "http", "allow_all_keys": true}"#,
        ),
    ];
    let shared = registry(config, db);
    let shared_registry = shared.read().await;
    assert_eq!(
        allow_all_keys_server_ids(&shared_registry),
        string_set(&["cfg-flag", "db-flag"])
    );
    let empty = registry(Vec::new(), Vec::new());
    let empty_registry = empty.read().await;
    assert!(allow_all_keys_server_ids(&empty_registry).is_empty());
}
