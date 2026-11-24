#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::Mutex;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, PermissionList};
use thiserror::Error;

use super::reads::{
    CachedLink, EndUserEntitlement, ObjectPermission, ObjectPermissionReads, OrgPermissionLink,
};
use super::record::McpAuth;

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

type Reply<T> = Ready<Result<T, StoreDown>>;
type Permission = Option<ObjectPermission>;
type LinkedId = Option<String>;

struct FakeReads {
    database: bool,
    ttl: f64,
    cached: Scripted<CachedLink>,
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
            cached: Scripted::Value(CachedLink::NoUsableLink),
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

    fn answer<T>(&self, entry: String, script: &Scripted<T>) -> Reply<T>
    where
        T: Clone,
    {
        self.record(entry);
        ready(script.resolve())
    }
}

#[expect(
    refining_impl_trait_internal,
    reason = "the fake answers ready futures, and the refined type never leaves this test module"
)]
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

    fn cached_link(&self, key: &str) -> Reply<CachedLink> {
        self.answer(format!("cached_link:{key}"), &self.cached)
    }

    fn store_link(&self, key: &str, value: &str, ttl: f64) -> Reply<()> {
        self.record(format!("store_link:{key}={value}@{ttl}"));
        ready(if self.store_fault {
            Err(StoreDown)
        } else {
            Ok(())
        })
    }

    fn permission(&self, object_permission_id: &str, check_db_only: bool) -> Reply<Permission> {
        self.answer(
            format!("permission:{object_permission_id}:{check_db_only}"),
            &self.permission,
        )
    }

    fn team_permission(&self, team_id: &str, check_db_only: bool) -> Reply<Permission> {
        self.answer(
            format!("team_permission:{team_id}:{check_db_only}"),
            &self.team,
        )
    }

    fn org_object_permission_link(&self, org_id: &str) -> Reply<OrgPermissionLink> {
        self.answer(format!("org_object_permission_link:{org_id}"), &self.org)
    }

    fn end_user_entitlement(&self, end_user_id: &str) -> Reply<Option<EndUserEntitlement>> {
        self.answer(
            format!("end_user_entitlement:{end_user_id}"),
            &self.end_user,
        )
    }

    fn user_object_permission_link(&self, user_id: &str, check_db_only: bool) -> Reply<LinkedId> {
        self.answer(
            format!("user_object_permission_link:{user_id}:{check_db_only}"),
            &self.user_link,
        )
    }

    fn agent_object_permission_link(&self, agent_id: &str) -> Reply<LinkedId> {
        self.answer(
            format!("agent_object_permission_link:{agent_id}"),
            &self.agent_link,
        )
    }

    fn resolve_toolset_grants(
        &self,
        toolset_ids: &[String],
        check_db_only: bool,
    ) -> Reply<BTreeMap<String, Vec<String>>> {
        self.answer(
            format!("resolve_toolset_grants:{toolset_ids:?}:{check_db_only}"),
            &self.toolset,
        )
    }
}

fn row(object_permission_id: &str) -> ObjectPermission {
    ObjectPermission {
        object_permission_id: String::from(object_permission_id),
        mcp_servers: PermissionList(Some(Vec::new())),
        mcp_access_groups: PermissionList(Some(Vec::new())),
        mcp_tool_permissions: None,
        vector_stores: PermissionList(Some(Vec::new())),
        agents: PermissionList(Some(Vec::new())),
        agent_access_groups: PermissionList(Some(Vec::new())),
        models: PermissionList(Some(Vec::new())),
        mcp_toolsets: None,
        blocked_tools: PermissionList(Some(Vec::new())),
        search_tools: PermissionList(Some(Vec::new())),
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

// a base with every key named: the ast-grep rules refuse ..McpAuth::default() in a literal
fn mcp_auth() -> McpAuth {
    McpAuth {
        user_id: None,
        team_id: None,
        org_id: None,
        end_user_id: None,
        agent_id: None,
        object_permission_id: None,
        object_permission: None,
        requires_fresh_policy: false,
    }
}

fn auth_from_json(json: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(json).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}
