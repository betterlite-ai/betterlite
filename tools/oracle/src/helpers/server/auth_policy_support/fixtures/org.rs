use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;

use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::resource_rows::organization::OrganizationTable;

// cache writes must not replace the row returned by the database fixture
pub(crate) struct FakeOrg {
    org: Option<OrganizationTable>,
    cached: Mutex<Option<String>>,
}

impl FakeOrg {
    pub(crate) fn seeded(org: Option<OrganizationTable>) -> Result<Self, serde_json::Error> {
        let serialized = org.as_ref().map(serde_json::to_string).transpose()?;
        Ok(Self {
            org,
            cached: Mutex::new(serialized),
        })
    }
}

#[async_trait]
impl OrgCache for FakeOrg {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        let serialized = {
            let stored = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
            stored.clone()
        };
        serialized
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|source| AuthCacheError::from(PayloadError::Json(source)))
    }

    async fn write_org(
        &self,
        _key: &str,
        org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        let serialized = serde_json::to_string(org).map_err(PayloadError::Json)?;
        let mut stored = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        *stored = Some(serialized);
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
        let Some(row) = self.org.as_ref() else {
            return Ok(None);
        };
        if row.organization_id.as_deref() != Some(org_id) {
            return Ok(None);
        }
        serde_json::to_string(row).map_or_else(|_| Ok(None), |text| Ok(Some(text)))
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
