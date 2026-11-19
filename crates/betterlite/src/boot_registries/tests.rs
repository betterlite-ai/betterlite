#![cfg(test)]

use std::sync::Arc;

use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;

use super::BootRegistries;
use crate::config_boot::error::ConfigBootError;
use crate::config_boot::non_llm::load_non_llm_configs;
use crate::config_boot::test_support::config_from_yaml;

fn registries() -> (Arc<AgentRegistry>, BootRegistries) {
    let registry = Arc::new(AgentRegistry::default());
    let ports = BootRegistries::new(Arc::clone(&registry));
    (registry, ports)
}

fn names(registry: &AgentRegistry) -> Vec<String> {
    registry
        .get_agent_list(None)
        .into_iter()
        .map(|agent| agent.agent_name)
        .collect()
}

fn step_of(error: ConfigBootError) -> (&'static str, String) {
    let text = error.to_string();
    let ConfigBootError::Step { step, source } = error else {
        panic!("expected a config boot step, got {text}");
    };
    (step, source.to_string())
}

const TWO_AGENTS: &str = "agents:\n  - agent_name: alpha\n    agent_card_params:\n      model: gpt-4o\n  - agent_name: beta\n    agent_card_params:\n      model: claude-sonnet-5-5\n";

#[tokio::test]
async fn boot_agents_registers_every_config_agent() {
    let config = config_from_yaml(TWO_AGENTS);
    let (registry, ports) = registries();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(
        names(&registry),
        vec![String::from("alpha"), String::from("beta")]
    );
    assert!(registry.get_agent_by_name("beta").is_some());
}

#[tokio::test]
async fn boot_agents_keeps_every_entry_field_type() {
    let config = config_from_yaml(
        "agents:\n  - agent_name: alpha\n    agent_card_params:\n      model: gpt-4o\n    litellm_params:\n      temperature: 0.5\n    tpm_limit: 5\n    rpm_limit: 0\n    session_tpm_limit: null\n    enabled: false\n    extra_headers:\n      - x-one\n",
    );
    let (registry, ports) = registries();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    let agents = registry.get_agent_list(None);
    assert_eq!(agents.len(), 1);
    let alpha = &agents[0];
    assert_eq!(alpha.tpm_limit, Some(5));
    assert_eq!(alpha.rpm_limit, Some(0));
    assert_eq!(alpha.session_tpm_limit, None);
    assert!(!alpha.enabled.0);
    assert_eq!(alpha.extra_headers, Some(vec![String::from("x-one")]));
    let temperature = alpha
        .litellm_params
        .as_ref()
        .and_then(|params| params.get("temperature"))
        .map(|raw| raw.get());
    assert_eq!(temperature, Some("0.5"));
}

#[tokio::test]
async fn boot_agents_loads_agent_list_when_agents_is_absent() {
    let config = config_from_yaml(
        "agent_list:\n  - agent_name: alpha\n    agent_card_params:\n      model: gpt-4o\n",
    );
    let (registry, ports) = registries();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(names(&registry), vec![String::from("alpha")]);
}

#[tokio::test]
async fn boot_agents_returns_the_loader_error() {
    let config = config_from_yaml(
        "agents:\n  - agent_name: gamma\n    agent_card_params:\n      model: gpt-4o\n    team_id: team\n",
    );
    let (registry, ports) = registries();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "agents failed");
    let (step, source) = step_of(error);
    assert_eq!(step, "agents");
    assert_eq!(source, "config agent entry has an unknown key `team_id`");
    assert!(names(&registry).is_empty());
}

#[tokio::test]
async fn boot_agents_non_list_is_refused_with_the_loader_error() {
    let config = config_from_yaml("agents: hello\n");
    let (registry, ports) = registries();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    let (step, source) = step_of(error);
    assert_eq!(step, "agents");
    assert_eq!(source, "agent_config must be a list of dictionaries");
    assert!(names(&registry).is_empty());
}

#[tokio::test]
async fn boot_agents_empty_list_boots() {
    let config = config_from_yaml("agents: []\n");
    let (registry, ports) = registries();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert!(names(&registry).is_empty());
}

#[tokio::test]
async fn boot_unbuilt_mcp_tools() {
    let config = config_from_yaml("mcp_tools:\n  - name: one\n");
    let (_registry, ports) = registries();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "mcp_tools failed");
    let (step, source) = step_of(error);
    assert_eq!(step, "mcp_tools");
    assert_eq!(
        source,
        "mcp_tools is set in the config, but its support is not built yet"
    );
}

#[tokio::test]
async fn boot_unbuilt_mcp_servers() {
    let config = config_from_yaml("mcp_servers:\n  - server_name: one\n");
    let (_registry, ports) = registries();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "mcp_servers failed");
    let (step, source) = step_of(error);
    assert_eq!(step, "mcp_servers");
    assert_eq!(
        source,
        "mcp_servers is set in the config, but its support is not built yet"
    );
}

#[tokio::test]
async fn boot_unbuilt_vector_store_registry() {
    let config = config_from_yaml("vector_store_registry:\n  - vector_store_name: store\n");
    let (_registry, ports) = registries();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "vector_store_registry failed");
    let (step, source) = step_of(error);
    assert_eq!(step, "vector_store_registry");
    assert_eq!(
        source,
        "vector_store_registry is set in the config, but its support is not built yet"
    );
}

#[tokio::test]
async fn boot_unbuilt_keys_left_empty() {
    let config = config_from_yaml("mcp_tools: []\nmcp_servers: {}\nvector_store_registry: []\n");
    let (registry, ports) = registries();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert!(names(&registry).is_empty());
}
