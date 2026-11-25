use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;

use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;

pub type ObjectPermission = LiteLLMObjectPermissionTable;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CachedLink {
    NoPermission,
    Id(String),
    // the adapter maps an empty cache entry and a cached value that is not a link to this one miss
    NoUsableLink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrgPermissionLink {
    NoPermission,
    Named(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndUserEntitlement {
    pub object_permission: Option<ObjectPermission>,
    pub object_permission_id: Option<String>,
}

pub trait ObjectPermissionReads: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn has_database(&self) -> bool;

    fn management_ttl(&self) -> f64;

    // the cache answers an id, the no-permission sentinel, or no usable link, and a reader re-reads the row on that miss
    fn cached_link(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<CachedLink, Self::Error>> + Send;

    fn store_link(
        &self,
        key: &str,
        value: &str,
        ttl: f64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    // get_object_permission: a non-fresh read answers None for a missing row and for a swallowed fault, while a fresh one propagates both
    fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send;

    fn team_permission(
        &self,
        team_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send;

    // get_org_object: Ok(NoPermission) when the org is absent, Err when the lookup failed for another reason
    fn org_object_permission_link(
        &self,
        org_id: &str,
    ) -> impl Future<Output = Result<OrgPermissionLink, Self::Error>> + Send;

    // get_end_user_object: None for an absent row or a swallowed fault, the relation and the id as stored otherwise
    fn end_user_entitlement(
        &self,
        end_user_id: &str,
    ) -> impl Future<Output = Result<Option<EndUserEntitlement>, Self::Error>> + Send;

    // get_user_object: Ok(None) when the row links no permission, Err when check_db_only makes the read propagate
    fn user_object_permission_link(
        &self,
        user_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send;

    fn agent_object_permission_link(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send;

    // resolve_toolset_tool_permissions: a non-fresh fault resolves to no grants, a fresh one propagates
    fn resolve_toolset_grants(
        &self,
        toolset_ids: &[String],
        check_db_only: bool,
    ) -> impl Future<Output = Result<BTreeMap<String, Vec<String>>, Self::Error>> + Send;
}
