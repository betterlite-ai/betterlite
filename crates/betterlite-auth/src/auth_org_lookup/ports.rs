use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_db::resource_rows::organization::OrganizationTable;

use crate::auth_cache::AuthCacheError;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_read::table::AuthReadError;

// a table read the deadline adapter refused or the database answered with
pub type StoreFailure = Box<AuthReadError<Infallible>>;
pub type StoreAnswer<Answer> = Result<Answer, StoreFailure>;

// async_get_cache and async_set_cache over B733, for the two management objects this module reads
#[async_trait]
pub trait OrgCache: Send + Sync {
    async fn read_org(
        &self,
        key: &str,
        local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError>;
    async fn write_org(
        &self,
        key: &str,
        org: &OrganizationTable,
        ttl: f64,
    ) -> Result<(), AuthCacheError>;
    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError>;
    async fn write_project(
        &self,
        key: &str,
        project: &CachedProject,
        ttl: f64,
    ) -> Result<(), AuthCacheError>;
    fn management_ttl(&self) -> f64;
}

// OrganizationRepository and ProjectRepository read JSON text, which the module decodes as its model
#[async_trait]
pub trait OrgStore: Send + Sync {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>>;
    async fn find_by_alias(&self, org_alias: &str) -> StoreAnswer<Vec<String>>;
    async fn find_project(&self, project_id: &str) -> StoreAnswer<Option<String>>;
}
