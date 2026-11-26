use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::organization::OrganizationTable;

// the cache trait's fallible write answer, so each impl names it once
type CacheWrite = Result<(), AuthCacheError>;

// the organization lookup a case seeds, either answering its one row or failing like an unreachable database
pub(crate) struct OrgWorld {
    organization: Option<OrganizationTable>,
    raises: bool,
    cached: Mutex<Option<OrganizationTable>>,
}

pub(crate) fn org_world(organization: Option<OrganizationTable>, raises: bool) -> Arc<OrgWorld> {
    Arc::new(OrgWorld {
        organization,
        raises,
        cached: Mutex::new(None),
    })
}

pub(crate) fn org_lookup(world: &Arc<OrgWorld>) -> OrgLookup {
    let cache: Arc<dyn OrgCache> = Arc::<OrgWorld>::clone(world);
    let store: Arc<dyn OrgStore> = Arc::<OrgWorld>::clone(world);
    OrgLookup {
        store: Some(store),
        cache,
        in_memory_ttl: 5.0_f64,
    }
}

#[async_trait]
impl OrgCache for OrgWorld {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        Ok(self
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn write_org(&self, _key: &str, org: &OrganizationTable, _ttl: f64) -> CacheWrite {
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(org.clone());
        Ok(())
    }

    async fn read_project(&self, _key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        Ok(None)
    }

    async fn write_project(&self, _key: &str, _project: &CachedProject, _ttl: f64) -> CacheWrite {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl OrgStore for OrgWorld {
    async fn find_org(&self, org_id: &str, _include_budget: bool) -> StoreAnswer<Option<String>> {
        if self.raises {
            return Err(Box::new(AuthReadError::Repository(DbError::CreateNull)));
        }
        let found = self
            .organization
            .clone()
            .filter(|row| row.organization_id.as_deref() == Some(org_id));
        Ok(found
            .map(|row| serde_json::to_string(&row).expect("the seeded organization serializes")))
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
