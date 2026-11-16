mod tests;

use std::future::Future;

use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRow;

use super::{RegistryReadThrough, Resync, ResyncError};

pub type ModelRow = ProxyModelTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectType {
    Models,
    Guardrails,
    Agents,
}

pub enum ModelKey<'key> {
    Name(&'key str),
    Id(&'key str),
}

pub trait DbObjects: Send + Sync {
    fn has_client(&self) -> bool;

    fn store_model_in_db(&self) -> bool;

    fn should_load(&self, object_type: ObjectType) -> bool;

    // LiteLLM's _db_backed_registries_enabled: a client, store_model_in_db exactly true and the type's should_load rule
    #[must_use]
    fn enabled(&self, object_type: ObjectType) -> bool {
        self.has_client() && self.store_model_in_db() && self.should_load(object_type)
    }
}

pub trait ModelSync: Send + Sync {
    fn find_rows(
        &self,
        by: ModelKey<'_>,
    ) -> impl Future<Output = Result<Vec<ModelRow>, ResyncError>> + Send;

    fn router_exists(&self) -> impl Future<Output = bool> + Send;

    fn add_deployments_from_db(&self) -> impl Future<Output = Result<bool, ResyncError>> + Send;

    fn add_rows_to_router(
        &self,
        rows: Vec<ModelRow>,
    ) -> impl Future<Output = Result<(), ResyncError>> + Send;
}

pub trait GuardrailSync: Send + Sync {
    fn find_active(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, ResyncError>> + Send;

    fn load_row(&self, row: GuardrailRow) -> impl Future<Output = bool> + Send;
}

pub trait AgentSync: Send + Sync {
    fn cached(&self, key: &str) -> impl Future<Output = bool> + Send;

    fn load_locked(&self, key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send;
}

pub struct ModelResync<S: ModelSync, D: DbObjects> {
    pub sync: S,
    pub db: D,
}

pub struct GuardrailResync<S: GuardrailSync, D: DbObjects> {
    pub sync: S,
    pub db: D,
}

pub struct AgentResync<S: AgentSync, D: DbObjects> {
    pub sync: S,
    pub db: D,
}

// the server state owns these three instead of LiteLLM's module-level singletons
pub struct RegistryReadThroughs<Models: Resync, Guardrails: Resync, Agents: Resync> {
    pub models: RegistryReadThrough<Models>,
    pub guardrails: RegistryReadThrough<Guardrails>,
    pub agents: RegistryReadThrough<Agents>,
}

impl<S: ModelSync, D: DbObjects> Resync for ModelResync<S, D> {
    async fn resync(&self, key: &str) -> Result<bool, ResyncError> {
        if !self.db.enabled(ObjectType::Models) {
            return Ok(false);
        }
        let mut rows = self.sync.find_rows(ModelKey::Name(key)).await?;
        if rows.is_empty() {
            rows = self.sync.find_rows(ModelKey::Id(key)).await?;
        }
        if rows.is_empty() {
            return Ok(false);
        }
        if self.sync.router_exists().await {
            self.sync.add_rows_to_router(rows).await?;
            return Ok(true);
        }
        self.sync.add_deployments_from_db().await
    }
}

impl<S: GuardrailSync, D: DbObjects> Resync for GuardrailResync<S, D> {
    async fn resync(&self, key: &str) -> Result<bool, ResyncError> {
        if !self.db.enabled(ObjectType::Guardrails) {
            return Ok(false);
        }
        let Some(row) = self.sync.find_active(key).await? else {
            return Ok(false);
        };
        Ok(self.sync.load_row(row).await)
    }
}

impl<S: AgentSync, D: DbObjects> Resync for AgentResync<S, D> {
    async fn resync(&self, key: &str) -> Result<bool, ResyncError> {
        if !self.db.enabled(ObjectType::Agents) {
            return Ok(false);
        }
        if self.sync.cached(key).await {
            Ok(true)
        } else {
            self.sync.load_locked(key).await
        }
    }
}
