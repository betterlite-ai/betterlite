pub mod fixtures;
pub mod postgres;

mod tests;

use std::future::Future;

use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use thiserror::Error;

use crate::access_group_sync::cache::{AccessGroupCache, invalidate_access_group_caches};

#[derive(Debug, Error)]
pub enum ModelSyncError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Cache(#[from] InvalidationError),
    #[error(transparent)]
    Lookup(#[from] LookupError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchedGroupRow {
    pub access_group_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentCountRow {
    pub deployment_count: i32,
}

// The statement a rename or delete runs, with the names its SQL binds as $1 and $2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelNameRewrite {
    Append { old_name: String, new_name: String },
    Replace { old_name: String, new_name: String },
    Remove { model_name: String },
}

// The write a rename follows: the deployment that changed and the names it moved between.
#[derive(Debug, Clone, Copy)]
pub struct ModelRename<'request> {
    pub model_id: &'request str,
    pub old_name: &'request str,
    pub new_name: &'request str,
}

pub trait ModelGrantDatabase: Send + Sync {
    fn backing_deployments(
        &self,
        model_name: &str,
    ) -> impl Future<Output = Result<Vec<DeploymentCountRow>, ModelSyncError>> + Send;

    fn rewrite_model_names(
        &self,
        rewrite: &ModelNameRewrite,
    ) -> impl Future<Output = Result<Vec<TouchedGroupRow>, ModelSyncError>> + Send;
}

// The maps a catalog-only lookup needs to satisfy: a sibling lookup resolves neither an alias nor a routing group.
#[derive(Default)]
struct EmptyCollaborators {
    aliases: AliasMap,
    groups: RoutingGroupIndex,
}

impl EmptyCollaborators {
    fn lookup<'lookup>(
        &'lookup self,
        catalog: &'lookup DeploymentCatalog,
        inferred_provider: &'lookup dyn Fn(&str) -> Option<String>,
    ) -> CatalogLookup<'lookup> {
        CatalogLookup {
            catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider,
        }
    }
}

// _config_sourced_sibling: another deployment under this id that the config still serves.
pub fn config_sourced_sibling(
    catalog: &DeploymentCatalog,
    deployment_id: &str,
    model_id: &str,
) -> Result<bool, LookupError> {
    let empties = EmptyCollaborators::default();
    let provider = |_model: &str| None;
    let lookup = empties.lookup(catalog, &provider);
    config_sourced(&lookup, deployment_id, model_id)
}

fn config_sourced(
    lookup: &CatalogLookup<'_>,
    deployment_id: &str,
    model_id: &str,
) -> Result<bool, LookupError> {
    if deployment_id == model_id {
        return Ok(false);
    }
    let Some(deployment) = lookup.get_deployment(deployment_id)? else {
        return Ok(false);
    };
    Ok(!matches!(
        deployment.model_info.get("db_model"),
        Some(Json::Bool(true))
    ))
}

// _served_by_a_config_deployment: any sibling of this name that the config still serves, or false without a router.
pub fn served_by_a_config_deployment(
    catalog: Option<&DeploymentCatalog>,
    model_name: &str,
    model_id: &str,
) -> Result<bool, LookupError> {
    let Some(router_catalog) = catalog else {
        return Ok(false);
    };
    let empties = EmptyCollaborators::default();
    let provider = |_model: &str| None;
    let lookup = empties.lookup(router_catalog, &provider);
    for id in lookup.get_model_ids(Some(model_name), false) {
        let Json::Text(deployment_id) = id else {
            continue;
        };
        if config_sourced(&lookup, &deployment_id, model_id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// still_backed: a config sibling short-circuits before the count, which does not exclude model_id itself.
pub async fn still_backed<Database>(
    database: &Database,
    catalog: Option<&DeploymentCatalog>,
    model_name: &str,
    model_id: &str,
) -> Result<bool, ModelSyncError>
where
    Database: ModelGrantDatabase + ?Sized,
{
    if served_by_a_config_deployment(catalog, model_name, model_id)? {
        return Ok(true);
    }
    let counts = database.backing_deployments(model_name).await?;
    Ok(counts.iter().any(|row| row.deployment_count > 0_i32))
}

async fn rewrite_groups<Database, Cache>(
    database: &Database,
    cache: &Cache,
    rewrite: ModelNameRewrite,
) -> Result<(), ModelSyncError>
where
    Database: ModelGrantDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    let rows = database.rewrite_model_names(&rewrite).await?;
    let touched: Vec<String> = rows.into_iter().map(|row| row.access_group_id).collect();
    invalidate_access_group_caches(cache, &touched).await?;
    Ok(())
}

pub async fn sync_groups_for_renamed_model<Database, Cache>(
    database: &Database,
    cache: &Cache,
    catalog: Option<&DeploymentCatalog>,
    rename: &ModelRename<'_>,
) -> Result<(), ModelSyncError>
where
    Database: ModelGrantDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    if rename.old_name == rename.new_name {
        return Ok(());
    }
    let backed = still_backed(database, catalog, rename.old_name, rename.model_id).await?;
    let old_name = rename.old_name.to_owned();
    let new_name = rename.new_name.to_owned();
    let rewrite = if backed {
        ModelNameRewrite::Append { old_name, new_name }
    } else {
        ModelNameRewrite::Replace { old_name, new_name }
    };
    rewrite_groups(database, cache, rewrite).await
}

pub async fn sync_groups_for_deleted_model<Database, Cache>(
    database: &Database,
    cache: &Cache,
    catalog: Option<&DeploymentCatalog>,
    model_id: &str,
    model_name: &str,
) -> Result<(), ModelSyncError>
where
    Database: ModelGrantDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    if still_backed(database, catalog, model_name, model_id).await? {
        return Ok(());
    }
    let rewrite = ModelNameRewrite::Remove {
        model_name: model_name.to_owned(),
    };
    rewrite_groups(database, cache, rewrite).await
}
