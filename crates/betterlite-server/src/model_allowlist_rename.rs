pub mod cache;
pub mod fixtures;
pub mod postgres;
pub mod statement;

mod tests;

use std::future::Future;

use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use thiserror::Error;

use self::cache::{AllowlistCache, cache_keys};
use self::statement::{AllowlistRewrite, TouchedRow};
use crate::access_group_models::{ModelGrantDatabase, ModelRename, ModelSyncError, still_backed};

#[derive(Debug, Error)]
pub enum ModelAllowlistError {
    #[error(transparent)]
    Grants(#[from] ModelSyncError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Cache(#[from] InvalidationError),
    #[error("the allowlist row names an unknown kind {kind}")]
    UnknownKind { kind: String },
}

pub trait AllowlistDatabase: Send + Sync {
    fn rewrite_allowlists(
        &self,
        rewrite: &AllowlistRewrite,
    ) -> impl Future<Output = Result<Vec<TouchedRow>, ModelAllowlistError>> + Send;
}

// sync_model_allowlists_for_renamed_model: the rename's old name stays granted while a sibling serves it.
pub async fn sync_model_allowlists_for_renamed_model<Grants, Allowlists, Cache>(
    grants: &Grants,
    allowlists: &Allowlists,
    cache: &Cache,
    catalog: Option<&DeploymentCatalog>,
    rename: &ModelRename<'_>,
) -> Result<(), ModelAllowlistError>
where
    Grants: ModelGrantDatabase + ?Sized,
    Allowlists: AllowlistDatabase + ?Sized,
    Cache: AllowlistCache + ?Sized,
{
    if rename.old_name == rename.new_name {
        return Ok(());
    }
    let rewrite = if still_backed(grants, catalog, rename.old_name, rename.model_id).await? {
        AllowlistRewrite::Append {
            old_name: rename.old_name.to_owned(),
            new_name: rename.new_name.to_owned(),
        }
    } else {
        AllowlistRewrite::Replace {
            old_name: rename.old_name.to_owned(),
            new_name: rename.new_name.to_owned(),
        }
    };
    let touched = allowlists.rewrite_allowlists(&rewrite).await?;
    let keys: Vec<String> = touched.iter().flat_map(cache_keys).collect();
    cache.evict_and_broadcast(&keys).await?;
    Ok(())
}
