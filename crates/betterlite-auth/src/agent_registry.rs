use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::num::TryFromIntError;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use tokio::sync::{Mutex, MutexGuard};

use crate::agent_policy::AgentResponse;

mod config_load;
mod grant_migration;
mod ids;
mod lookup;
mod migration_retry;
pub mod python_json;
mod reconcile_lock;
mod registry_ops;
mod reload;
mod tests;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GrantMigrationResult {
    pub rewritten: usize,
    pub missed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentRegistryError {
    #[error("agent_config must be a list of dictionaries")]
    NotAListOfDictionaries,
    #[error("config agent entry has an unknown key `{key}`")]
    UnknownConfigKey { key: String },
    #[error("config agent entry must not set `agent_id`")]
    ConfigAgentId,
    #[error("config agent entry field `agent_name` must be a string")]
    AgentNameType,
    #[error("config agent entry field `agent_card_params` must be an object")]
    AgentCardType,
    #[error("config agent entry field `{path}` is invalid: {source}")]
    Record {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("migration affected row count did not fit in usize")]
    RowCount {
        #[source]
        source: TryFromIntError,
    },
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Default)]
pub struct AgentRegistry {
    state: RwLock<State>,
    reconcile_lock: Mutex<()>,
}

#[derive(Default)]
struct State {
    agent_list: Vec<AgentResponse>,
    config_agents: Vec<Json>,
    legacy_to_stable: BTreeMap<String, String>,
}

// The workspace denies multiple_inherent_impl, so each area's logic is a free function in its own module and this trait is the type's one impl block.
pub trait AgentRegistryApi {
    fn register_agent(&self, agent: AgentResponse);
    fn deregister_agent(&self, agent_name: &str);
    fn reset_agent_list(&self);
    #[must_use]
    fn get_agent_list(&self, agent_names: Option<&[String]>) -> Vec<AgentResponse>;
    #[must_use]
    fn get_public_agent_list(&self, public_agent_groups: Option<&[String]>) -> Vec<AgentResponse>;
    #[must_use]
    fn ids_for_agent(&self, agent_id: &str) -> BTreeSet<String>;
    #[must_use]
    fn stable_agent_id(&self, agent_id: &str) -> String;
    fn load_agents_from_config(
        &self,
        agent_config: Option<&[Json]>,
    ) -> Result<(), AgentRegistryError>;
    fn load_agents_from_db_and_config(
        &self,
        agent_config: Option<&[Json]>,
        db_agents: Option<&[AgentResponse]>,
    ) -> Result<Vec<AgentResponse>, AgentRegistryError>;
    #[must_use]
    fn get_agent_by_id(&self, agent_id: &str) -> Option<AgentResponse>;
    #[must_use]
    fn get_agent_by_name(&self, agent_name: &str) -> Option<AgentResponse>;
    fn reconcile(&self) -> impl Future<Output = MutexGuard<'_, ()>> + Send;
    fn migrate_legacy_grant_ids(
        &self,
        store: &PostgresStore,
    ) -> impl Future<Output = Result<GrantMigrationResult, AgentRegistryError>> + Send;
    fn run_grant_id_migration(&self, store: &PostgresStore) -> impl Future<Output = ()> + Send;
}

impl AgentRegistryApi for AgentRegistry {
    fn register_agent(&self, agent: AgentResponse) {
        let mut state = self.state_mut();
        state.agent_list.push(agent);
    }

    fn deregister_agent(&self, agent_name: &str) {
        let mut state = self.state_mut();
        state
            .agent_list
            .retain(|agent| agent.agent_name != agent_name);
    }

    fn reset_agent_list(&self) {
        let mut state = self.state_mut();
        state.agent_list.clear();
    }

    fn get_agent_list(&self, agent_names: Option<&[String]>) -> Vec<AgentResponse> {
        registry_ops::get_agent_list(self, agent_names)
    }

    fn get_public_agent_list(&self, public_agent_groups: Option<&[String]>) -> Vec<AgentResponse> {
        registry_ops::get_public_agent_list(self, public_agent_groups.unwrap_or_default())
    }

    fn ids_for_agent(&self, agent_id: &str) -> BTreeSet<String> {
        ids::ids_for_agent(self, agent_id)
    }

    fn stable_agent_id(&self, agent_id: &str) -> String {
        ids::stable_agent_id(self, agent_id)
    }

    fn load_agents_from_config(
        &self,
        agent_config: Option<&[Json]>,
    ) -> Result<(), AgentRegistryError> {
        config_load::load_agents_from_config(self, agent_config)
    }

    fn load_agents_from_db_and_config(
        &self,
        agent_config: Option<&[Json]>,
        db_agents: Option<&[AgentResponse]>,
    ) -> Result<Vec<AgentResponse>, AgentRegistryError> {
        reload::load_agents_from_db_and_config(self, agent_config, db_agents)
    }

    fn get_agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        lookup::get_agent_by_id(self, agent_id)
    }

    fn get_agent_by_name(&self, agent_name: &str) -> Option<AgentResponse> {
        lookup::get_agent_by_name(self, agent_name)
    }

    async fn reconcile(&self) -> MutexGuard<'_, ()> {
        self.reconcile_lock.lock().await
    }

    fn migrate_legacy_grant_ids(
        &self,
        store: &PostgresStore,
    ) -> impl Future<Output = Result<GrantMigrationResult, AgentRegistryError>> + Send {
        grant_migration::migrate_legacy_grant_ids(self, store)
    }

    fn run_grant_id_migration(&self, store: &PostgresStore) -> impl Future<Output = ()> + Send {
        migration_retry::run_grant_id_migration(self, store)
    }
}

impl AgentRegistry {
    fn state(&self) -> RwLockReadGuard<'_, State> {
        match self.state.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn state_mut(&self) -> RwLockWriteGuard<'_, State> {
        match self.state.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
