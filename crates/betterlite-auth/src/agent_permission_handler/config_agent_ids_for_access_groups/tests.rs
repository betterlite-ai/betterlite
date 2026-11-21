#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::AgentRegistryError;

#[test]
fn a_config_agent_cannot_declare_an_access_group() {
    let config: Vec<Json> = vec![
        serde_json::from_str(
            r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"agent_access_groups":["g1"]}"#,
        )
        .unwrap(),
    ];
    let registry = AgentRegistry::default();
    let error = registry.load_agents_from_config(Some(&config)).unwrap_err();
    let AgentRegistryError::UnknownConfigKey { key } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(key, "agent_access_groups");
    assert!(registry.get_agent_list(None).is_empty());
}
