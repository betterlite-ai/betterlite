use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError as StoreError;

use super::store::ObjectPermissionStore;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

// litellm/proxy/auth/auth_checks.py: get_object_permission's caller-visible 403 text
pub const MISSING_PERMISSION_MESSAGE: &str = "Referenced object permission does not exist";
pub const MISSING_PERMISSION_STATUS: u16 = 403;
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct PermissionCacheError(#[from] AuthCacheError);
pub type PermissionAnswer = Result<Option<LiteLLMObjectPermissionTable>, PermissionCacheError>;
// async_get_cache/async_set_cache over B733; read/write dodge clippy's same_name_method on AuthCache
#[async_trait]
pub trait ObjectPermissionCache: Send + Sync {
    async fn read(&self, key: &str) -> PermissionAnswer;
    async fn write(
        &self,
        key: &str,
        permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError>;
    fn management_ttl(&self) -> f64;
}
#[async_trait]
impl<Tier> ObjectPermissionCache for AuthCache<Tier>
where
    Tier: AuthCacheTier,
{
    async fn read(&self, key: &str) -> PermissionAnswer {
        self.async_get_typed::<LiteLLMObjectPermissionTable>(key, false)
            .await
            .map_err(|source| PermissionCacheError::from(AuthCacheError::from(source)))
    }
    async fn write(
        &self,
        key: &str,
        permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(key), permission, options)
            .await
            .map_err(PermissionCacheError::from)
    }
    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
}
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDatabase,
    #[error("the object permission cache failed")]
    Cache(#[source] PermissionCacheError),
    #[error("Referenced object permission does not exist")]
    MissingInDatabaseOnly,
    #[error(transparent)]
    Lookup(#[from] DbLookupError),
    #[error("the object permission repository failed")]
    Repository(#[source] StoreError<TableError>),
    #[error("the object permission row did not validate")]
    Validation(#[source] serde_json::Error),
}
impl PermissionError {
    // B715's decorator records a DB failure only for the faults it can name, so pass it those
    #[must_use]
    pub fn database_fault(&self) -> Option<DbError> {
        match self {
            Self::Lookup(DbLookupError::Database(error)) => Some(error.clone()),
            Self::Repository(error) => repository_fault(error),
            Self::NoDatabase
            | Self::Cache(_)
            | Self::MissingInDatabaseOnly
            | Self::Lookup(
                DbLookupError::Deadline(_) | DbLookupError::Elapsed(_) | DbLookupError::Task(_),
            )
            | Self::Validation(_) => None,
        }
    }
}
// LiteLLM's prisma client raises a plain Exception for a missing client and PrismaError otherwise
fn repository_fault(error: &StoreError<TableError>) -> Option<DbError> {
    if matches!(error, StoreError::NoDbConnected) {
        return None;
    }
    let error_type = if is_native_transport_error(error) {
        ErrorType::Connect
    } else {
        ErrorType::Prisma
    };
    Some(DbError {
        error_type,
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}
#[derive(Clone, Copy)]
pub struct PermissionStores<'source> {
    pub normal: &'source dyn ObjectPermissionStore,
    pub writer: &'source dyn ObjectPermissionStore,
}
pub struct ObjectPermissionSource<'source> {
    pub cache: &'source dyn ObjectPermissionCache,
    pub stores: Option<PermissionStores<'source>>,
}
// auth_checks.py get_object_permission: cache first, then one keyed database read, then backfill
pub async fn get_object_permission(
    source: &ObjectPermissionSource<'_>,
    object_permission_id: &str,
    check_db_only: bool,
) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
    let Some(stores) = source.stores else {
        return Err(PermissionError::NoDatabase);
    };
    let cache = source.cache;
    let key = CacheKey::ObjectPermissionId(object_permission_id).render();
    if !check_db_only {
        let cached = cache.read(&key).await.map_err(PermissionError::Cache)?;
        if let Some(permission) = cached {
            return Ok(Some(permission));
        }
    }
    let outcome = lookup_permission(cache, stores, &key, object_permission_id, check_db_only).await;
    if check_db_only {
        return outcome;
    }
    Ok(outcome.ok().flatten())
}
async fn lookup_permission(
    cache: &dyn ObjectPermissionCache,
    stores: PermissionStores<'_>,
    key: &str,
    object_permission_id: &str,
    check_db_only: bool,
) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
    let store = if check_db_only {
        stores.writer
    } else {
        stores.normal
    };
    let Some(row) = store.find_permission(object_permission_id).await? else {
        return if check_db_only {
            Err(PermissionError::MissingInDatabaseOnly)
        } else {
            Ok(None)
        };
    };
    let wire = serde_json::to_value(row).map_err(PermissionError::Validation)?;
    let permission = serde_json::from_value::<LiteLLMObjectPermissionTable>(wire)
        .map_err(PermissionError::Validation)?;
    let ttl = cache.management_ttl();
    cache
        .write(key, &permission, ttl)
        .await
        .map_err(PermissionError::Cache)?;
    Ok(Some(permission))
}
