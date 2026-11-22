use std::collections::BTreeSet;
use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi};
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{AgentKillSwitchConfig, AgentKillSwitchResult};
use thiserror::Error;

use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::AuditEventSink;
use crate::kill_switch_management::client::KillSwitchHttpClient;
use crate::kill_switch_management::delivery::{DEFAULT_TIMEOUT, fire_kill_switch};

// an agent as the registry or the agents table returns it: the id, the name and the stored kill_switch JSON
#[derive(Clone, Debug)]
pub struct KnownAgent {
    pub agent_id: String,
    pub agent_name: String,
    pub kill_switch: Option<AgentJson>,
}

// agents_table(prisma_client).find_unique: None for an agent that has no row, as a config.yaml agent has none
#[async_trait]
pub trait AgentRowLookup: Send + Sync {
    async fn agent_row(&self, agent_id: &str) -> Result<Option<KnownAgent>, PortFailure>;
}

// AGENT_REGISTRY.get_agent_by_id and AGENT_REGISTRY.ids_for_agent
pub trait AgentRegistryLookup: Send + Sync {
    fn agent_by_id(&self, agent_id: &str) -> Option<KnownAgent>;
    fn ids_for_agent(&self, agent_id: &str) -> BTreeSet<String>;
}

// litellm.public_agent_groups, None until a route sets it the way LiteLLM's module global starts out
pub trait PublicGroupStore: Send + Sync {
    fn groups(&self) -> Option<Vec<String>>;
    fn set_groups(&self, groups: Vec<String>);
}

// proxy_config.save_config with litellm_settings.public_agent_groups set to the route's new list
#[async_trait]
pub trait PublicGroupConfigWriter: Send + Sync {
    async fn save_public_groups(&self, groups: &[String]) -> Result<(), PortFailure>;
}

// fire_kill_switch: the mount supplies the client and the route callers get whatever came back
#[async_trait]
pub trait KillSwitchTrigger: Send + Sync {
    async fn fire(&self, agent_id: &str, config: &AgentKillSwitchConfig) -> AgentKillSwitchResult;
}

// default_kill_switch_audit_log_writer, which B065's create_audit_log_for_update implements
#[async_trait]
pub trait KillSwitchAuditWriter: Send + Sync {
    async fn write(&self, record: AuditLogRecord);
}

// the mount's port implementations live in crates this one cannot name, so a failure carries its cause boxed
#[derive(Debug, Error)]
#[error("{source}")]
pub struct PortFailure {
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
}

impl AgentRegistryLookup for AgentRegistry {
    fn agent_by_id(&self, agent_id: &str) -> Option<KnownAgent> {
        let agent = self.get_agent_by_id(agent_id)?;
        let kill_switch = agent
            .kill_switch
            .as_ref()
            .and_then(|config| serde_json::to_value(config).ok());
        Some(KnownAgent {
            agent_id: agent.agent_id,
            agent_name: agent.agent_name,
            kill_switch,
        })
    }

    fn ids_for_agent(&self, agent_id: &str) -> BTreeSet<String> {
        AgentRegistryApi::ids_for_agent(self, agent_id)
    }
}

// the real trigger: fire_kill_switch with the client the mount wired and its default timeout
pub struct DeliveringTrigger<Client> {
    pub client: Client,
}

#[async_trait]
impl<Client> KillSwitchTrigger for DeliveringTrigger<Client>
where
    Client: KillSwitchHttpClient,
{
    async fn fire(&self, agent_id: &str, config: &AgentKillSwitchConfig) -> AgentKillSwitchResult {
        fire_kill_switch(agent_id, config, &self.client, DEFAULT_TIMEOUT).await
    }
}

#[async_trait]
impl KillSwitchAuditWriter for AuditEventSink {
    async fn write(&self, record: AuditLogRecord) {
        self.create_audit_log_for_update(record).await;
    }
}
