use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;

use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::resource_rows::organization::OrganizationTable;

// the org cache the lookup reads: it answers the seeded row, and a write replaces it
pub(crate) struct FakeOrg {
    pub(crate) org: Mutex<Option<OrganizationTable>>,
}

#[async_trait]
impl OrgCache for FakeOrg {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        let org = {
            let stored = self.org.lock().unwrap_or_else(PoisonError::into_inner);
            stored.clone()
        };
        Ok(org)
    }

    async fn write_org(
        &self,
        _key: &str,
        org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        {
            let mut stored = self.org.lock().unwrap_or_else(PoisonError::into_inner);
            *stored = Some(org.clone())
        };
        Ok(())
    }

    async fn read_project(&self, _key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        Ok(None)
    }

    async fn write_project(
        &self,
        _key: &str,
        _project: &CachedProject,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl OrgStore for FakeOrg {
    async fn find_org(&self, org_id: &str, _include_budget: bool) -> StoreAnswer<Option<String>> {
        let cached = {
            let stored = self.org.lock().unwrap_or_else(PoisonError::into_inner);
            stored.clone()
        };
        let Some(row) = cached else {
            return Ok(None);
        };
        if row.organization_id.as_deref() != Some(org_id) {
            return Ok(None);
        }
        serde_json::to_string(&row).map_or_else(|_| Ok(None), |text| Ok(Some(text)))
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
