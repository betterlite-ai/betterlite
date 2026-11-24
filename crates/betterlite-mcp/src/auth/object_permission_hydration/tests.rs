#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, PermissionList};
use thiserror::Error;

use super::reads::{
    CachedLink, EndUserEntitlement, ObjectPermission, ObjectPermissionReads, OrgPermissionLink,
};

mod agent_tests;
mod grants_tests;
mod hydration_tests;
mod user_tests;

#[derive(Debug, Error)]
#[error("the permission store is down")]
struct StoreDown;

#[derive(Clone, Debug)]
enum Scripted<T> {
    Value(T),
    Fault,
}

impl<T: Clone> Scripted<T> {
    fn resolve(&self) -> Result<T, StoreDown> {
        match self {
            Self::Value(value) => Ok(value.clone()),
            Self::Fault => Err(StoreDown),
        }
    }
}

struct FakeReads {
    database: bool,
    ttl: f64,
    cached: Scripted<Option<CachedLink>>,
    store_fault: bool,
    permission: Scripted<Option<ObjectPermission>>,
    team: Scripted<Option<ObjectPermission>>,
    org: Scripted<OrgPermissionLink>,
    end_user: Scripted<Option<EndUserEntitlement>>,
    user_link: Scripted<Option<String>>,
    agent_link: Scripted<Option<String>>,
    toolset: Scripted<BTreeMap<String, Vec<String>>>,
    log: Mutex<Vec<String>>,
}

impl FakeReads {
    fn standard() -> Self {
        Self {
            database: true,
            ttl: 12.5,
            cached: Scripted::Value(None),
            store_fault: false,
            permission: Scripted::Value(None),
            team: Scripted::Value(None),
            org: Scripted::Value(OrgPermissionLink::NoPermission),
            end_user: Scripted::Value(None),
            user_link: Scripted::Value(None),
            agent_link: Scripted::Value(None),
            toolset: Scripted::Value(BTreeMap::new()),
            log: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl ObjectPermissionReads for FakeReads {
    type Error = StoreDown;

    fn has_database(&self) -> bool {
        self.record(String::from("has_database"));
        self.database
    }

    fn management_ttl(&self) -> f64 {
        self.record(String::from("management_ttl"));
        self.ttl
    }

    fn cached_link(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<CachedLink>, Self::Error>> + Send {
        self.record(format!("cached_link:{key}"));
        ready(self.cached.resolve())
    }

    fn store_link(
        &self,
        key: &str,
        value: &str,
        ttl: f64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.record(format!("store_link:{key}={value}@{ttl}"));
        let outcome = if self.store_fault {
            Err(StoreDown)
        } else {
            Ok(())
        };
        ready(outcome)
    }

    fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        self.record(format!("permission:{object_permission_id}:{check_db_only}"));
        ready(self.permission.resolve())
    }

    fn team_permission(
        &self,
        team_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        self.record(format!("team_permission:{team_id}:{check_db_only}"));
        ready(self.team.resolve())
    }

    fn org_object_permission_link(
        &self,
        org_id: &str,
    ) -> impl Future<Output = Result<OrgPermissionLink, Self::Error>> + Send {
        self.record(format!("org_object_permission_link:{org_id}"));
        ready(self.org.resolve())
    }

    fn end_user_entitlement(
        &self,
        end_user_id: &str,
    ) -> impl Future<Output = Result<Option<EndUserEntitlement>, Self::Error>> + Send {
        self.record(format!("end_user_entitlement:{end_user_id}"));
        ready(self.end_user.resolve())
    }

    fn user_object_permission_link(
        &self,
        user_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        self.record(format!(
            "user_object_permission_link:{user_id}:{check_db_only}"
        ));
        ready(self.user_link.resolve())
    }

    fn agent_object_permission_link(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        self.record(format!("agent_object_permission_link:{agent_id}"));
        ready(self.agent_link.resolve())
    }

    fn resolve_toolset_grants(
        &self,
        toolset_ids: &[String],
        check_db_only: bool,
    ) -> impl Future<Output = Result<BTreeMap<String, Vec<String>>, Self::Error>> + Send {
        self.record(format!(
            "resolve_toolset_grants:{toolset_ids:?}:{check_db_only}"
        ));
        ready(self.toolset.resolve())
    }
}

fn row(object_permission_id: &str) -> ObjectPermission {
    ObjectPermission {
        object_permission_id: String::from(object_permission_id),
        mcp_servers: PermissionList(None),
        mcp_access_groups: PermissionList(None),
        mcp_tool_permissions: None,
        vector_stores: PermissionList(None),
        agents: PermissionList(None),
        agent_access_groups: PermissionList(None),
        models: PermissionList(None),
        mcp_toolsets: None,
        blocked_tools: PermissionList(None),
        search_tools: PermissionList(None),
        mcp_tool_search_enabled: None,
        skills: None,
    }
}

fn row_with_toolsets(object_permission_id: &str, toolset_ids: &[&str]) -> ObjectPermission {
    let mut permission = row(object_permission_id);
    permission.mcp_toolsets = Some(toolset_ids.iter().map(|id| String::from(*id)).collect());
    permission
}

fn grants_of(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| String::from(*name)).collect()
}

fn expected(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|entry| String::from(*entry)).collect()
}

fn auth_from_json(json: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(json).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}
