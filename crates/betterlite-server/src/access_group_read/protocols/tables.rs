use std::future::Future;

use super::error::AccessGroupStoreError;
use super::records::{
    AccessGroupChanges, AccessGroupRecord, AgentRecord, KeyRecord, NewAccessGroup, TeamRecord,
};

// The Prisma where clauses delete and _teams_touching send, kept as the filters they become
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamWhere {
    IdIn(Vec<String>),
    CarryingAny(Vec<String>),
    CarryingAnyOrIdIn {
        groups: Vec<String>,
        ids: Vec<String>,
    },
}

pub trait AccessGroupTable: Send + Sync {
    fn find_by_id(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<Option<AccessGroupRecord>, AccessGroupStoreError>> + Send;

    fn find_by_name(
        &self,
        access_group_name: &str,
    ) -> impl Future<Output = Result<Option<AccessGroupRecord>, AccessGroupStoreError>> + Send;

    fn list_newest_first(
        &self,
    ) -> impl Future<Output = Result<Vec<AccessGroupRecord>, AccessGroupStoreError>> + Send;

    fn create(
        &self,
        new: &NewAccessGroup,
    ) -> impl Future<Output = Result<AccessGroupRecord, AccessGroupStoreError>> + Send;

    fn update(
        &self,
        access_group_id: &str,
        changes: &AccessGroupChanges,
    ) -> impl Future<Output = Result<AccessGroupRecord, AccessGroupStoreError>> + Send;

    fn delete(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<(), AccessGroupStoreError>> + Send;
}

pub trait TeamTable: Send + Sync {
    fn find(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Option<TeamRecord>, AccessGroupStoreError>> + Send;

    fn find_many(
        &self,
        filter: &TeamWhere,
    ) -> impl Future<Output = Result<Vec<TeamRecord>, AccessGroupStoreError>> + Send;

    fn set_access_group_ids(
        &self,
        team_id: &str,
        ids: &[String],
    ) -> impl Future<Output = Result<(), AccessGroupStoreError>> + Send;
}

pub trait KeyTable: Send + Sync {
    fn find(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<KeyRecord>, AccessGroupStoreError>> + Send;

    fn find_carrying(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<Vec<KeyRecord>, AccessGroupStoreError>> + Send;

    fn set_access_group_ids(
        &self,
        token: &str,
        ids: &[String],
    ) -> impl Future<Output = Result<(), AccessGroupStoreError>> + Send;
}

pub trait AgentTable: Send + Sync {
    fn find_carrying(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<Vec<AgentRecord>, AccessGroupStoreError>> + Send;

    fn set_access_group_ids(
        &self,
        agent_id: &str,
        ids: &[String],
    ) -> impl Future<Output = Result<(), AccessGroupStoreError>> + Send;
}

pub trait AccessGroupTx: Send + Sync {
    fn litellm_accessgrouptable(&self) -> impl AccessGroupTable;

    fn litellm_teamtable(&self) -> impl TeamTable;

    fn litellm_verificationtoken(&self) -> impl KeyTable;

    fn litellm_agentstable(&self) -> impl AgentTable;
}
