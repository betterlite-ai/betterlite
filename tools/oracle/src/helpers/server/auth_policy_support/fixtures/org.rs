use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;

use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::resource_rows::organization::OrganizationTable;

// the org cache the lookup reads: it answers the seeded row, and a write replaces it
pub(crate) struct FakeOrg {
    org: Mutex<Option<OrganizationTable>>,
    projects: Mutex<BTreeMap<String, CachedProject>>,
}

impl FakeOrg {
    pub(crate) fn new(org: Option<OrganizationTable>) -> Self {
        Self {
            org: Mutex::new(org),
            projects: Mutex::new(BTreeMap::new()),
        }
    }
}

#[async_trait]
impl OrgCache for FakeOrg {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        Ok(self.org.lock().ok().and_then(|guard| guard.clone()))
    }

    async fn write_org(
        &self,
        _key: &str,
        org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        if let Ok(mut guard) = self.org.lock() {
            *guard = Some(org.clone());
        }
        Ok(())
    }

    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        let projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(projects.get(key).cloned())
    }

    async fn write_project(
        &self,
        key: &str,
        project: &CachedProject,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        let mut projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        projects.insert(key.to_owned(), project.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl OrgStore for FakeOrg {
    async fn find_org(&self, org_id: &str, _include_budget: bool) -> StoreAnswer<Option<String>> {
        let Some(org) = self.org.lock().ok().and_then(|guard| guard.clone()) else {
            return Ok(None);
        };
        if org.organization_id.as_deref() != Some(org_id) {
            return Ok(None);
        }
        match serde_json::to_string(&org) {
            Ok(text) => Ok(Some(text)),
            Err(_) => Ok(None),
        }
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
