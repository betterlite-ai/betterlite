use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::resource_rows::organization::OrganizationTable;

use crate::auth_cache::AuthCacheError;
use crate::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use crate::auth_prefetch::rows::CachedProject;

// the B879 store and cache over one seeded row; a None row is the database answer that get_org_object raises NotFound for
#[derive(Default)]
pub(crate) struct OrgWorld {
    pub(crate) found: Option<String>,
    pub(crate) lookups: Mutex<Vec<(String, bool)>>,
    writes: Mutex<Vec<String>>,
}

impl OrgWorld {
    pub(crate) fn with_row(row: &OrganizationTable) -> Self {
        Self {
            found: Some(serde_json::to_string(row).expect("the organization row serializes")),
            lookups: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn lookups(&self) -> Vec<(String, bool)> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn writes(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl OrgCache for OrgWorld {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        Ok(None)
    }

    async fn write_org(
        &self,
        key: &str,
        _org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
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
        60.0
    }
}

#[async_trait]
impl OrgStore for OrgWorld {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((org_id.to_owned(), include_budget));
        Ok(self.found.clone())
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}
