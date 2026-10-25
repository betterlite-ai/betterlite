#![cfg(test)]

use std::collections::BTreeMap;

use crate::search::{BaseSearchConfig, CredentialRequest};

#[test]
fn uses_ordered_injected_lookup() {
    let config = BaseSearchConfig;
    let values = BTreeMap::from([(String::from("SECOND"), String::from("server-key"))]);
    let lookup = |name: &str| values.get(name).cloned();
    let result = config
        .resolve_server_api_key(
            &CredentialRequest {
                caller_api_key: None,
                caller_api_base: Some("https://search.example/v1"),
                key_env_vars: &["FIRST", "SECOND"],
                base_env_var: None,
                default_api_base: Some("https://search.example/v1"),
            },
            &lookup,
        )
        .unwrap();
    assert_eq!(result.as_deref(), Some("server-key"));
}
