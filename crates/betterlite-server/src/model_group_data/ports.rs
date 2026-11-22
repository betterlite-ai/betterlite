use std::future::Future;

use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::transport::InvalidationTransport;
use betterlite_router::catalog_updates::catalog::CatalogHandle;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use super::errors::GroupDataError;
use super::types::{DeploymentRow, GroupBudgetRow};

// _BudgetInclude: whether the read joins LiteLLM_BudgetTable into each link row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetInclude {
    pub litellm_budget_table: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupBudgetCreate {
    pub access_group_name: String,
    pub budget_id: Option<String>,
    pub created_by: String,
    pub updated_by: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupBudgetUpdate {
    pub budget_id: Option<String>,
    pub updated_by: String,
}

// _model_table and _model_access_group_budget_table: the two tables this module reads and writes.
pub trait GroupDatabase: Send + Sync {
    fn deployment_find_many(
        &self,
        model_names: Option<&[String]>,
    ) -> impl Future<Output = Result<Vec<DeploymentRow>, GroupDataError>> + Send;

    fn deployment_find_unique(
        &self,
        model_id: &str,
    ) -> impl Future<Output = Result<Option<DeploymentRow>, GroupDataError>> + Send;

    fn update_model_info(
        &self,
        model_id: &str,
        model_info: &str,
    ) -> impl Future<Output = Result<(), GroupDataError>> + Send;

    fn budget_find_unique(
        &self,
        access_group: &str,
        include: BudgetInclude,
    ) -> impl Future<Output = Result<Option<GroupBudgetRow>, GroupDataError>> + Send;

    fn budget_find_many(
        &self,
        include: BudgetInclude,
    ) -> impl Future<Output = Result<Vec<GroupBudgetRow>, GroupDataError>> + Send;

    fn budget_upsert(
        &self,
        create: &GroupBudgetCreate,
        update: &GroupBudgetUpdate,
        include: BudgetInclude,
    ) -> impl Future<Output = Result<GroupBudgetRow, GroupDataError>> + Send;

    fn budget_delete(
        &self,
        access_group: &str,
    ) -> impl Future<Output = Result<Option<GroupBudgetRow>, GroupDataError>> + Send;
}

// _auth_cache: the cache eviction every budget write must run.
pub trait ModelAccessGroupCache: Send + Sync {
    fn evict_group_cache_keys(&self, cache_keys: &[String]) -> impl Future<Output = ()> + Send;
}

impl ModelAccessGroupCache for AuthInvalidation {
    async fn evict_group_cache_keys(&self, cache_keys: &[String]) {
        InvalidationTransport::evict_and_broadcast(self.transport.as_ref(), cache_keys, self).await;
    }
}

// proxy_server.llm_router read fresh: the names this pod serves now, or none while no router is installed; a reload shows up in the next call.
pub trait ModelNameSource: Send + Sync {
    fn model_names(&self) -> Result<Option<Vec<String>>, GroupDataError>;
}

// The live router state: the catalog behind the reload swap and the alias and routing-group maps the config serves, all borrowed so every call reads what the router holds now.
pub struct LiveModelNames<'state> {
    pub catalog: &'state CatalogHandle,
    pub model_group_alias: &'state AliasMap,
    pub routing_groups: &'state RoutingGroupIndex,
}

impl ModelNameSource for LiveModelNames<'_> {
    fn model_names(&self) -> Result<Option<Vec<String>>, GroupDataError> {
        let catalog = self.catalog.load();
        let provider = |_model: &str| None;
        let lookup = CatalogLookup {
            catalog: catalog.as_ref(),
            model_group_alias: self.model_group_alias,
            routing_groups: self.routing_groups,
            inferred_provider: &provider,
        };
        Ok(Some(lookup.get_model_names(None)?))
    }
}
