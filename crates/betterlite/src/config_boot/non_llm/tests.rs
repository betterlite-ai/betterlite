#![cfg(test)]

use std::io::Error;
use std::path::Path;
use std::sync::Mutex;

use betterlite_config::sections::settings::JsonValue;

use super::{NonLlmRegistries, load_non_llm_configs};
use crate::config_boot::error::ConfigBootError;
use crate::config_boot::test_support::config_from_yaml;
use crate::job_scheduler::BoxFuture;

#[derive(Default)]
struct RecordingRegistries {
    calls: Mutex<Vec<String>>,
    fail_at: Option<&'static str>,
}

impl RecordingRegistries {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn failing_at(step: &'static str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail_at: Some(step),
        }
    }

    fn record(&self, call: String) -> Result<(), ConfigBootError> {
        let failed = self.fail_at.is_some_and(|step| call.starts_with(step));
        self.calls.lock().unwrap().push(call);
        if failed {
            return Err(ConfigBootError::Step {
                step: "test registry",
                source: Box::new(Error::other("the test registry failed")),
            });
        }
        Ok(())
    }
}

fn tool_record(tools: &JsonValue, config_file_path: Option<&Path>) -> String {
    let count = tools.as_array().map_or(0, Vec::len);
    let path =
        config_file_path.map_or_else(|| "unset".to_owned(), |path| path.display().to_string());
    format!("mcp_tools:{count}:{path}")
}

fn servers_record(servers: &JsonValue, mcp_aliases: Option<&JsonValue>) -> String {
    format!(
        "mcp_servers:{}:{}",
        servers.as_array().map_or(0, Vec::len),
        mcp_aliases.is_some()
    )
}

impl NonLlmRegistries for RecordingRegistries {
    fn mcp_tools<'config>(
        &'config self,
        tools: &'config JsonValue,
        config_file_path: Option<&'config Path>,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        Box::pin(async move { self.record(tool_record(tools, config_file_path)) })
    }

    fn agents<'config>(
        &'config self,
        agents: &'config JsonValue,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        let call = format!("agents:{}", agents.as_array().map_or(0, Vec::len));
        Box::pin(async move { self.record(call) })
    }

    fn mcp_servers<'config>(
        &'config self,
        servers: &'config JsonValue,
        mcp_aliases: Option<&'config JsonValue>,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        Box::pin(async move { self.record(servers_record(servers, mcp_aliases)) })
    }

    fn vector_stores<'config>(
        &'config self,
        registry: &'config JsonValue,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        let call = format!("vector_stores:{}", registry.as_array().map_or(0, Vec::len));
        Box::pin(async move { self.record(call) })
    }
}

fn all_registries() -> &'static str {
    "mcp_tools:\n  - name: one\nagents:\n  - name: a\n  - name: b\nmcp_servers:\n  - name: srv\nvector_store_registry:\n  - name: store\n"
}

#[tokio::test]
async fn the_registries_load_in_litellm_order() {
    let config = config_from_yaml(all_registries());
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, Some(Path::new("/config.yaml")), &ports)
        .await
        .unwrap();
    assert_eq!(
        ports.calls(),
        vec![
            "mcp_tools:1:/config.yaml".to_owned(),
            "agents:2".to_owned(),
            "mcp_servers:1:false".to_owned(),
            "vector_stores:1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn the_agents_key_wins_over_agent_list() {
    let config = config_from_yaml("agents:\n  - name: a\nagent_list:\n  - name: b\n  - name: c\n");
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(ports.calls(), vec!["agents:1".to_owned()]);
}

#[tokio::test]
async fn agent_list_loads_when_agents_is_absent() {
    let config = config_from_yaml("agent_list:\n  - name: a\n  - name: b\n");
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(ports.calls(), vec!["agents:2".to_owned()]);
}

#[tokio::test]
async fn an_empty_agents_list_still_reaches_the_registry() {
    let config = config_from_yaml("agents: []\n");
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(ports.calls(), vec!["agents:0".to_owned()]);
}

#[tokio::test]
async fn an_explicit_null_agents_key_skips_the_fallback() {
    let config = config_from_yaml("agents:\nagent_list:\n  - name: a\n");
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn falsy_registry_keys_are_skipped() {
    let config = config_from_yaml(
        "mcp_tools: []\nmcp_servers: {}\nvector_store_registry: []\nworker_registry: []\n",
    );
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn mcp_servers_receives_the_configured_aliases() {
    let config = config_from_yaml(
        "mcp_servers:\n  - name: srv\nlitellm_settings:\n  mcp_aliases:\n    srv: alias\n",
    );
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(ports.calls(), vec!["mcp_servers:1:true".to_owned()]);
}

#[tokio::test]
async fn mcp_servers_without_litellm_settings_passes_no_aliases() {
    let config = config_from_yaml("mcp_servers:\n  - name: srv\n");
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, None, &ports).await.unwrap();
    assert_eq!(ports.calls(), vec!["mcp_servers:1:false".to_owned()]);
}

#[tokio::test]
async fn a_non_mapping_litellm_settings_is_refused() {
    let config = config_from_yaml("mcp_servers:\n  - name: srv\nlitellm_settings: hello\n");
    let ports = RecordingRegistries::default();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "config field 'litellm_settings' must be a mapping"
    );
}

#[tokio::test]
async fn worker_registry_fails_with_the_premium_text() {
    let config = config_from_yaml("worker_registry:\n  - name: worker\n");
    let ports = RecordingRegistries::default();
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(
        text.starts_with("Trying to use `worker_registry`You must be a LiteLLM Enterprise user")
    );
    assert!(text.contains("https://www.litellm.ai/enterprise#trial"));
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn a_failing_port_stops_the_remaining_steps() {
    let config = config_from_yaml(all_registries());
    let ports = RecordingRegistries::failing_at("mcp_servers");
    let error = load_non_llm_configs(&config, None, &ports)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("test registry failed"));
    assert_eq!(
        ports.calls(),
        vec![
            "mcp_tools:1:unset".to_owned(),
            "agents:2".to_owned(),
            "mcp_servers:1:false".to_owned(),
        ]
    );
}

#[tokio::test]
async fn the_config_file_path_reaches_mcp_tools() {
    let config = config_from_yaml("mcp_tools:\n  - name: one\n");
    let ports = RecordingRegistries::default();
    load_non_llm_configs(&config, Some(Path::new("/etc/litellm.yaml")), &ports)
        .await
        .unwrap();
    assert_eq!(
        ports.calls(),
        vec!["mcp_tools:1:/etc/litellm.yaml".to_owned()]
    );
}
