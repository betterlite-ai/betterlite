#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, PermissionList};
use thiserror::Error;

use super::error::{
    EntitlementError, USER_POLICY_UNAVAILABLE_STATUS, UnloadableEntitlementError,
    UserPermissionError,
};
use super::grants::{
    key_or_team_declares_toolsets, toolset_tool_permissions, toolset_tools_for_server,
    union_tool_grants,
};
use super::hydration::load_named_object_permission;
use super::principal::{
    agent_object_permission, agent_object_permission_id, end_user_object_permission,
    key_object_permission_hydrated, org_object_permission, team_object_permission,
    user_object_permission, user_object_permission_id,
};
use super::reads::{
    CachedLink, EndUserEntitlement, ObjectPermission, ObjectPermissionReads, OrgPermissionLink,
};
use super::record::{MCP_ALL_TOOLS_WILDCARD, McpAuth};

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

#[tokio::test]
async fn load_named_returns_the_row_and_passes_the_fresh_flag() {
    let reads = FakeReads {
        permission: Scripted::Value(Some(row("op-1"))),
        ..FakeReads::standard()
    };
    let outcome: Result<ObjectPermission, UnloadableEntitlementError<StoreDown>> =
        load_named_object_permission(&reads, "agent \"a1\"", "op-1", true).await;
    assert_eq!(outcome.unwrap(), row("op-1"));
    assert_eq!(reads.calls(), expected(&["permission:op-1:true"]));
}

#[tokio::test]
async fn load_named_errors_when_the_row_is_absent() {
    let reads = FakeReads::standard();
    let error = load_named_object_permission(&reads, "end user \"eu\"", "op-1", false)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("end user \"eu\" names object_permission_id \"op-1\"")
    );
    assert!(error.source().is_none());
}

#[tokio::test]
async fn load_named_keeps_the_read_fault_as_the_cause() {
    let reads = FakeReads {
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let error = load_named_object_permission(&reads, "agent \"a1\"", "op-1", false)
        .await
        .unwrap_err();
    assert_eq!(
        error.source().unwrap().to_string(),
        "the permission store is down"
    );
}

#[test]
fn mcp_auth_decodes_the_ids_the_relation_and_the_fresh_mark() {
    let auth = auth_from_json(
        r#"{"user_id": "u1", "team_id": "t1", "org_id": "o1", "end_user_id": "eu", "agent_id": "a1", "object_permission_id": "op-key", "object_permission": {"object_permission_id": "op-key"}}"#,
    );
    let record = McpAuth::from_auth(&auth).unwrap();
    assert_eq!(record.user_id.as_deref(), Some("u1"));
    assert_eq!(record.team_id.as_deref(), Some("t1"));
    assert_eq!(record.org_id.as_deref(), Some("o1"));
    assert_eq!(record.end_user_id.as_deref(), Some("eu"));
    assert_eq!(record.agent_id.as_deref(), Some("a1"));
    assert_eq!(record.object_permission_id.as_deref(), Some("op-key"));
    assert_eq!(record.object_permission, Some(row("op-key")));
    assert!(!record.requires_fresh_policy);
}

#[test]
fn mcp_auth_defaults_absent_fields() {
    let auth = auth_from_json("{}");
    assert_eq!(McpAuth::from_auth(&auth).unwrap(), McpAuth::default());
}

#[tokio::test]
async fn team_permission_is_none_without_a_team_a_database_or_a_real_team() {
    let reads = FakeReads::standard();
    assert!(
        team_object_permission(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        team_id: Some(String::from("t1")),
        ..McpAuth::default()
    };
    assert!(
        team_object_permission(&offline, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));

    let dashboard = FakeReads::standard();
    let ui_auth = McpAuth {
        team_id: Some(String::from("litellm-dashboard")),
        ..McpAuth::default()
    };
    assert!(
        team_object_permission(&dashboard, &ui_auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(dashboard.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn team_permission_returns_the_hydrated_row_with_the_fresh_flag() {
    let reads = FakeReads {
        team: Scripted::Value(Some(row("op-team"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        team_id: Some(String::from("t1")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    assert_eq!(
        team_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-team"))
    );
    assert_eq!(
        reads.calls(),
        expected(&["has_database", "team_permission:t1:true"])
    );
}

#[tokio::test]
async fn key_permission_returns_the_preloaded_relation_without_a_read() {
    let reads = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission: Some(row("op-loaded")),
        ..McpAuth::default()
    };
    assert_eq!(
        key_object_permission_hydrated(&reads, &auth).await.unwrap(),
        Some(row("op-loaded"))
    );
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn key_permission_is_none_without_an_id_or_a_database() {
    let reads = FakeReads::standard();
    assert!(
        key_object_permission_hydrated(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        ..McpAuth::default()
    };
    assert!(
        key_object_permission_hydrated(&offline, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn key_permission_loads_the_named_row_with_the_fresh_flag() {
    let reads = FakeReads {
        permission: Scripted::Value(Some(row("op-key"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        ..McpAuth::default()
    };
    assert_eq!(
        key_object_permission_hydrated(&reads, &auth).await.unwrap(),
        Some(row("op-key"))
    );
    assert_eq!(
        reads.calls(),
        expected(&["has_database", "permission:op-key:false"])
    );

    let fresh = FakeReads {
        permission: Scripted::Value(Some(row("op-key"))),
        ..FakeReads::standard()
    };
    let fresh_auth = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    assert_eq!(
        key_object_permission_hydrated(&fresh, &fresh_auth)
            .await
            .unwrap(),
        Some(row("op-key"))
    );
    assert_eq!(
        fresh.calls(),
        expected(&["has_database", "permission:op-key:true"])
    );
}

#[tokio::test]
async fn org_permission_is_none_without_an_org_a_database_or_a_named_row() {
    let reads = FakeReads::standard();
    assert!(
        org_object_permission(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let auth = McpAuth {
        org_id: Some(String::from("o1")),
        ..McpAuth::default()
    };
    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    assert!(
        org_object_permission(&offline, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));

    let unnamed = FakeReads::standard();
    assert!(
        org_object_permission(&unnamed, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        unnamed.calls(),
        expected(&["has_database", "org_object_permission_link:o1"])
    );
}

#[tokio::test]
async fn org_permission_loads_the_named_row_with_the_fresh_flag() {
    let reads = FakeReads {
        org: Scripted::Value(OrgPermissionLink::Named(String::from("op-org"))),
        permission: Scripted::Value(Some(row("op-org"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        org_id: Some(String::from("o1")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    assert_eq!(
        org_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-org"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "org_object_permission_link:o1",
            "permission:op-org:true",
        ])
    );
}

#[tokio::test]
async fn org_permission_is_unloadable_when_the_named_row_is_absent_or_unreadable() {
    let auth = McpAuth {
        org_id: Some(String::from("o1")),
        ..McpAuth::default()
    };
    let absent = FakeReads {
        org: Scripted::Value(OrgPermissionLink::Named(String::from("op-org"))),
        ..FakeReads::standard()
    };
    let error = org_object_permission(&absent, &auth).await.unwrap_err();
    assert!(matches!(&error, EntitlementError::Unloadable(_)));
    assert!(error.to_string().contains("op-org"));

    let unreadable = FakeReads {
        org: Scripted::Value(OrgPermissionLink::Named(String::from("op-org"))),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let fault = org_object_permission(&unreadable, &auth).await.unwrap_err();
    assert!(matches!(&fault, EntitlementError::Unloadable(_)));
    assert!(fault.source().is_some());
}

#[tokio::test]
async fn org_permission_propagates_a_link_read_fault() {
    let reads = FakeReads {
        org: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        org_id: Some(String::from("o1")),
        ..McpAuth::default()
    };
    let error = org_object_permission(&reads, &auth).await.unwrap_err();
    assert!(matches!(&error, EntitlementError::Read { .. }));
}

#[tokio::test]
async fn end_user_permission_is_none_without_an_id_a_row_or_a_named_permission() {
    let reads = FakeReads::standard();
    assert!(
        end_user_object_permission(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let auth = McpAuth {
        end_user_id: Some(String::from("eu")),
        ..McpAuth::default()
    };
    let absent = FakeReads::standard();
    assert!(
        end_user_object_permission(&absent, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(absent.calls(), expected(&["end_user_entitlement:eu"]));

    let unnamed = FakeReads {
        end_user: Scripted::Value(Some(EndUserEntitlement {
            object_permission: None,
            object_permission_id: None,
        })),
        ..FakeReads::standard()
    };
    assert!(
        end_user_object_permission(&unnamed, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(unnamed.calls(), expected(&["end_user_entitlement:eu"]));
}

#[tokio::test]
async fn end_user_permission_swallows_a_lookup_fault() {
    let reads = FakeReads {
        end_user: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        end_user_id: Some(String::from("eu")),
        ..McpAuth::default()
    };
    assert!(
        end_user_object_permission(&reads, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.calls(), expected(&["end_user_entitlement:eu"]));
}

#[tokio::test]
async fn end_user_permission_prefers_the_loaded_relation() {
    let reads = FakeReads {
        end_user: Scripted::Value(Some(EndUserEntitlement {
            object_permission: Some(row("op-relation")),
            object_permission_id: Some(String::from("op-named")),
        })),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        end_user_id: Some(String::from("eu")),
        ..McpAuth::default()
    };
    assert_eq!(
        end_user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-relation"))
    );
    assert_eq!(reads.calls(), expected(&["end_user_entitlement:eu"]));
}

#[tokio::test]
async fn end_user_permission_loads_the_named_row() {
    let reads = FakeReads {
        end_user: Scripted::Value(Some(EndUserEntitlement {
            object_permission: None,
            object_permission_id: Some(String::from("op-named")),
        })),
        permission: Scripted::Value(Some(row("op-named"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        end_user_id: Some(String::from("eu")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    assert_eq!(
        end_user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-named"))
    );
    assert_eq!(
        reads.calls(),
        expected(&["end_user_entitlement:eu", "permission:op-named:true"])
    );
}

#[tokio::test]
async fn end_user_permission_is_unloadable_when_the_named_row_is_absent_or_unreadable() {
    let auth = McpAuth {
        end_user_id: Some(String::from("eu")),
        ..McpAuth::default()
    };
    let absent = FakeReads {
        end_user: Scripted::Value(Some(EndUserEntitlement {
            object_permission: None,
            object_permission_id: Some(String::from("op-named")),
        })),
        ..FakeReads::standard()
    };
    let error = end_user_object_permission(&absent, &auth)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("end user \"eu\""));
    assert!(error.source().is_none());

    let unreadable = FakeReads {
        end_user: Scripted::Value(Some(EndUserEntitlement {
            object_permission: None,
            object_permission_id: Some(String::from("op-named")),
        })),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let fault = end_user_object_permission(&unreadable, &auth)
        .await
        .unwrap_err();
    assert!(fault.source().is_some());
}

#[tokio::test]
async fn user_permission_is_none_without_a_user_or_a_database() {
    let reads = FakeReads::standard();
    assert!(
        user_object_permission(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    assert!(
        user_object_permission(&offline, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn user_permission_is_none_when_the_user_links_none() {
    let reads = FakeReads::standard();
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    assert!(
        user_object_permission(&reads, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "user_object_permission_link:u1:false",
            "management_ttl",
            "store_link:user_object_permission_id:u1=__user_no_mcp_permission__@12.5",
        ])
    );
}

#[tokio::test]
async fn user_permission_loads_the_linked_row() {
    let reads = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Value(Some(row("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    assert_eq!(
        user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-user"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "user_object_permission_link:u1:false",
            "management_ttl",
            "store_link:user_object_permission_id:u1=op-user@12.5",
            "permission:op-user:false",
        ])
    );
}

#[tokio::test]
async fn user_permission_uses_a_cached_link_without_a_row_read() {
    let reads = FakeReads {
        cached: Scripted::Value(Some(CachedLink::Id(String::from("op-cached")))),
        permission: Scripted::Value(Some(row("op-cached"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    assert_eq!(
        user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-cached"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "permission:op-cached:false",
        ])
    );
}

#[tokio::test]
async fn user_permission_returns_none_for_the_cached_sentinel() {
    let reads = FakeReads {
        cached: Scripted::Value(Some(CachedLink::NoPermission)),
        user_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    assert!(
        user_object_permission(&reads, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.calls(),
        expected(&["has_database", "cached_link:user_object_permission_id:u1"])
    );
}

#[tokio::test]
async fn user_permission_treats_a_cached_non_string_as_a_miss() {
    let reads = FakeReads {
        cached: Scripted::Value(Some(CachedLink::NotAPermissionLink)),
        user_link: Scripted::Value(Some(String::from("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    let outcome = user_object_permission(&reads, &auth).await.unwrap();
    assert_eq!(outcome.unwrap().object_permission_id, "op-user");
    assert!(
        reads
            .calls()
            .iter()
            .any(|call| call.as_str() == "user_object_permission_link:u1:false")
    );
}

#[tokio::test]
async fn user_permission_skips_the_cache_when_the_policy_must_be_fresh() {
    let reads = FakeReads {
        cached: Scripted::Fault,
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Value(Some(row("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    assert_eq!(
        user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-user"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "user_object_permission_link:u1:true",
            "management_ttl",
            "store_link:user_object_permission_id:u1=op-user@12.5",
            "permission:op-user:true",
        ])
    );
}

#[tokio::test]
async fn user_permission_answers_none_on_a_link_or_cache_fault() {
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    let link_fault = FakeReads {
        user_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert!(
        user_object_permission(&link_fault, &auth)
            .await
            .unwrap()
            .is_none()
    );

    let cache_fault = FakeReads {
        cached: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert!(
        user_object_permission(&cache_fault, &auth)
            .await
            .unwrap()
            .is_none()
    );

    let write_fault = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        store_fault: true,
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert!(
        user_object_permission(&write_fault, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        write_fault.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "user_object_permission_link:u1:false",
            "management_ttl",
            "store_link:user_object_permission_id:u1=op-user@12.5",
        ])
    );
}

#[tokio::test]
async fn user_permission_answers_503_when_a_fresh_link_or_cache_write_fails() {
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    let link_fault = FakeReads {
        user_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    let error = user_object_permission(&link_fault, &auth)
        .await
        .unwrap_err();
    assert!(matches!(&error, UserPermissionError::Unavailable { .. }));
    assert_eq!(error.to_string(), "User policy is unavailable");

    let write_fault = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        store_fault: true,
        ..FakeReads::standard()
    };
    let write_error = user_object_permission(&write_fault, &auth)
        .await
        .unwrap_err();
    assert!(matches!(
        &write_error,
        UserPermissionError::Unavailable { .. }
    ));
}

#[tokio::test]
async fn user_permission_is_a_value_error_when_the_named_row_is_absent() {
    let reads = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    let error = user_object_permission(&reads, &auth).await.unwrap_err();
    assert!(matches!(&error, UserPermissionError::Missing { .. }));
    assert_eq!(
        error.to_string(),
        "user \"u1\" names object_permission_id \"op-user\" which could not be loaded"
    );
}

#[tokio::test]
async fn user_permission_propagates_a_permission_read_fault() {
    let reads = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..McpAuth::default()
    };
    let error = user_object_permission(&reads, &auth).await.unwrap_err();
    assert!(matches!(&error, UserPermissionError::Read { .. }));
}

#[test]
fn user_policy_unavailable_status_is_503() {
    assert_eq!(USER_POLICY_UNAVAILABLE_STATUS, 503);
}

#[tokio::test]
async fn agent_id_prefers_a_cached_sentinel_or_id_over_the_row() {
    let sentinel = FakeReads {
        cached: Scripted::Value(Some(CachedLink::NoPermission)),
        agent_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&sentinel, "a1").await, None);
    assert_eq!(
        sentinel.calls(),
        expected(&["cached_link:agent_object_permission_id:a1"])
    );

    let cached = FakeReads {
        cached: Scripted::Value(Some(CachedLink::Id(String::from("op-agent")))),
        agent_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(
        agent_object_permission_id(&cached, "a1").await,
        Some(String::from("op-agent"))
    );
}

#[tokio::test]
async fn agent_id_reads_the_row_and_caches_the_link() {
    let reads = FakeReads {
        agent_link: Scripted::Value(Some(String::from("op-agent"))),
        ..FakeReads::standard()
    };
    assert_eq!(
        agent_object_permission_id(&reads, "a1").await,
        Some(String::from("op-agent"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "cached_link:agent_object_permission_id:a1",
            "agent_object_permission_link:a1",
            "management_ttl",
            "store_link:agent_object_permission_id:a1=op-agent@12.5",
        ])
    );
}

#[tokio::test]
async fn agent_id_caches_the_sentinel_when_the_agent_links_none() {
    let reads = FakeReads::standard();
    assert_eq!(agent_object_permission_id(&reads, "a1").await, None);
    assert_eq!(
        reads.calls(),
        expected(&[
            "cached_link:agent_object_permission_id:a1",
            "agent_object_permission_link:a1",
            "management_ttl",
            "store_link:agent_object_permission_id:a1=__agent_no_mcp_permission__@12.5",
        ])
    );
}

#[tokio::test]
async fn agent_id_treats_a_cached_non_string_as_a_miss() {
    let reads = FakeReads {
        cached: Scripted::Value(Some(CachedLink::NotAPermissionLink)),
        agent_link: Scripted::Value(Some(String::from("op-agent"))),
        ..FakeReads::standard()
    };
    assert_eq!(
        agent_object_permission_id(&reads, "a1").await,
        Some(String::from("op-agent"))
    );
    assert!(
        reads
            .calls()
            .iter()
            .any(|call| call.as_str() == "agent_object_permission_link:a1")
    );
}

#[tokio::test]
async fn agent_id_answers_none_on_a_cache_row_or_write_fault() {
    let cache_fault = FakeReads {
        cached: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&cache_fault, "a1").await, None);
    assert_eq!(
        cache_fault.calls(),
        expected(&["cached_link:agent_object_permission_id:a1"])
    );

    let row_fault = FakeReads {
        agent_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&row_fault, "a1").await, None);

    let write_fault = FakeReads {
        agent_link: Scripted::Value(Some(String::from("op-agent"))),
        store_fault: true,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&write_fault, "a1").await, None);
}

#[tokio::test]
async fn agent_permission_is_none_without_an_agent_or_a_database() {
    let reads = FakeReads::standard();
    assert!(
        agent_object_permission(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        ..McpAuth::default()
    };
    assert!(
        agent_object_permission(&offline, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn agent_permission_is_none_when_the_agent_links_none() {
    let reads = FakeReads::standard();
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        ..McpAuth::default()
    };
    assert!(
        agent_object_permission(&reads, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:agent_object_permission_id:a1",
            "agent_object_permission_link:a1",
            "management_ttl",
            "store_link:agent_object_permission_id:a1=__agent_no_mcp_permission__@12.5",
        ])
    );
}

#[tokio::test]
async fn agent_permission_loads_the_linked_row_with_the_fresh_flag() {
    let reads = FakeReads {
        cached: Scripted::Value(Some(CachedLink::Id(String::from("op-agent")))),
        permission: Scripted::Value(Some(row("op-agent"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        requires_fresh_policy: true,
        ..McpAuth::default()
    };
    assert_eq!(
        agent_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-agent"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:agent_object_permission_id:a1",
            "permission:op-agent:true",
        ])
    );
}

#[tokio::test]
async fn agent_permission_is_unloadable_when_the_named_row_is_absent_or_unreadable() {
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        ..McpAuth::default()
    };
    let absent = FakeReads {
        cached: Scripted::Value(Some(CachedLink::Id(String::from("op-agent")))),
        ..FakeReads::standard()
    };
    let error = agent_object_permission(&absent, &auth).await.unwrap_err();
    assert!(error.to_string().contains("agent \"a1\""));
    assert!(error.source().is_none());

    let unreadable = FakeReads {
        cached: Scripted::Value(Some(CachedLink::Id(String::from("op-agent")))),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let fault = agent_object_permission(&unreadable, &auth)
        .await
        .unwrap_err();
    assert!(fault.source().is_some());
}

#[tokio::test]
async fn toolset_grants_are_empty_without_a_row_or_declared_toolsets() {
    let reads = FakeReads::standard();
    assert_eq!(
        toolset_tool_permissions(&reads, None, false).await.unwrap(),
        BTreeMap::new()
    );
    assert_eq!(
        toolset_tool_permissions(&reads, Some(&row("op")), false)
            .await
            .unwrap(),
        BTreeMap::new()
    );
    assert_eq!(
        toolset_tool_permissions(&reads, Some(&row_with_toolsets("op", &[])), false)
            .await
            .unwrap(),
        BTreeMap::new()
    );
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn toolset_grants_resolve_the_declared_ids_with_the_fresh_flag() {
    let reads = FakeReads {
        toolset: Scripted::Value(BTreeMap::from([
            (String::from("srv-a"), grants_of(&["read", "write"])),
            (String::from("srv-b"), grants_of(&["list"])),
        ])),
        ..FakeReads::standard()
    };
    let permission = row_with_toolsets("op", &["ts-1", "ts-2"]);
    let outcome = toolset_tool_permissions(&reads, Some(&permission), true)
        .await
        .unwrap();
    assert_eq!(outcome.get("srv-a"), Some(&grants_of(&["read", "write"])));
    assert_eq!(outcome.get("srv-b"), Some(&grants_of(&["list"])));
    assert_eq!(
        reads.calls(),
        expected(&["resolve_toolset_grants:[\"ts-1\", \"ts-2\"]:true"])
    );
}

#[tokio::test]
async fn toolset_grants_are_unloadable_when_the_resolver_returns_nothing() {
    let reads = FakeReads::standard();
    let permission = row_with_toolsets("op", &["ts-1"]);
    let error = toolset_tool_permissions(&reads, Some(&permission), false)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Unloadable(_)));
    assert!(error.to_string().contains("ts-1"));
    assert_eq!(
        reads.calls(),
        expected(&["resolve_toolset_grants:[\"ts-1\"]:false"])
    );
}

#[tokio::test]
async fn toolset_grants_propagate_a_resolver_fault() {
    let reads = FakeReads {
        toolset: Scripted::Fault,
        ..FakeReads::standard()
    };
    let permission = row_with_toolsets("op", &["ts-1"]);
    let error = toolset_tool_permissions(&reads, Some(&permission), false)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Read { .. }));
}

#[tokio::test]
async fn toolset_tools_for_a_server_select_its_grants() {
    let reads = FakeReads {
        toolset: Scripted::Value(BTreeMap::from([
            (String::from("srv-a"), grants_of(&["read"])),
            (String::from("srv-b"), Vec::new()),
        ])),
        ..FakeReads::standard()
    };
    let permission = row_with_toolsets("op", &["ts-1"]);
    assert_eq!(
        toolset_tools_for_server(&reads, Some(&permission), "srv-a", false)
            .await
            .unwrap(),
        Some(grants_of(&["read"]))
    );
    assert_eq!(
        toolset_tools_for_server(&reads, Some(&permission), "srv-b", false)
            .await
            .unwrap(),
        Some(Vec::new())
    );
    assert!(
        toolset_tools_for_server(&reads, Some(&permission), "srv-c", false)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn toolset_tools_are_none_or_unloadable_without_grants() {
    let reads = FakeReads::standard();
    assert!(
        toolset_tools_for_server(&reads, None, "srv-a", false)
            .await
            .unwrap()
            .is_none()
    );
    let permission = row_with_toolsets("op", &["ts-1"]);
    let error = toolset_tools_for_server(&reads, Some(&permission), "srv-a", false)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Unloadable(_)));
}

#[test]
fn wildcard_constant_is_the_star() {
    assert_eq!(MCP_ALL_TOOLS_WILDCARD, "*");
}

#[test]
fn union_is_none_when_the_direct_grant_holds_the_wildcard() {
    let direct = grants_of(&["*", "read"]);
    let via = grants_of(&["write"]);
    assert!(union_tool_grants(Some(&direct), Some(&via)).is_none());
}

#[test]
fn union_is_none_when_neither_source_restricts() {
    assert!(union_tool_grants(None, None).is_none());
}

#[test]
fn union_keeps_a_single_source_and_merges_both() {
    let direct = grants_of(&["b", "a"]);
    let via = grants_of(&["a", "c"]);
    assert_eq!(
        union_tool_grants(Some(&direct), None),
        Some(grants_of(&["a", "b"]))
    );
    assert_eq!(
        union_tool_grants(None, Some(&via)),
        Some(grants_of(&["a", "c"]))
    );
    assert_eq!(
        union_tool_grants(Some(&direct), Some(&via)),
        Some(grants_of(&["a", "b", "c"]))
    );
}

#[test]
fn union_keeps_an_empty_direct_grant_as_a_restriction() {
    let empty: Vec<String> = Vec::new();
    assert_eq!(union_tool_grants(Some(&empty), None), Some(Vec::new()));
    assert_eq!(union_tool_grants(None, Some(&empty)), Some(Vec::new()));
}

#[test]
fn union_ignores_the_wildcard_in_a_toolset_grant() {
    let via = grants_of(&["*"]);
    assert_eq!(union_tool_grants(None, Some(&via)), Some(grants_of(&["*"])));
}

#[test]
fn union_matches_the_wildcard_exactly() {
    let doubled = grants_of(&["**"]);
    assert_eq!(
        union_tool_grants(Some(&doubled), None),
        Some(grants_of(&["**"]))
    );
}

#[tokio::test]
async fn nothing_declares_toolsets_without_an_auth() {
    let reads = FakeReads::standard();
    assert!(!key_or_team_declares_toolsets(&reads, None).await);
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn a_key_relation_with_toolsets_declares_them() {
    let reads = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission: Some(row_with_toolsets("op", &["ts-1"])),
        ..McpAuth::default()
    };
    assert!(key_or_team_declares_toolsets(&reads, Some(&auth)).await);
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn a_key_without_toolsets_and_without_a_team_declares_nothing() {
    let reads = FakeReads::standard();
    let auth = McpAuth {
        object_permission: Some(row("op")),
        ..McpAuth::default()
    };
    assert!(!key_or_team_declares_toolsets(&reads, Some(&auth)).await);
    assert!(reads.calls().is_empty());
}

#[tokio::test]
async fn an_unhydrated_key_is_loaded_before_the_toolset_check() {
    let reads = FakeReads {
        permission: Scripted::Value(Some(row_with_toolsets("op-key", &["ts-1"]))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        ..McpAuth::default()
    };
    assert!(key_or_team_declares_toolsets(&reads, Some(&auth)).await);
    assert_eq!(
        reads.calls(),
        expected(&["has_database", "permission:op-key:false"])
    );
}

#[tokio::test]
async fn a_team_declares_its_toolsets() {
    let auth = McpAuth {
        team_id: Some(String::from("t1")),
        ..McpAuth::default()
    };
    let declaring = FakeReads {
        team: Scripted::Value(Some(row_with_toolsets("op-team", &["ts-1"]))),
        ..FakeReads::standard()
    };
    assert!(key_or_team_declares_toolsets(&declaring, Some(&auth)).await);
    assert_eq!(
        declaring.calls(),
        expected(&["has_database", "team_permission:t1:false"])
    );

    let outlined = FakeReads {
        team: Scripted::Value(Some(row("op-team"))),
        ..FakeReads::standard()
    };
    assert!(!key_or_team_declares_toolsets(&outlined, Some(&auth)).await);

    let absent = FakeReads::standard();
    assert!(!key_or_team_declares_toolsets(&absent, Some(&auth)).await);

    let dashboard = FakeReads::standard();
    let ui_auth = McpAuth {
        team_id: Some(String::from("litellm-dashboard")),
        ..McpAuth::default()
    };
    assert!(!key_or_team_declares_toolsets(&dashboard, Some(&ui_auth)).await);
    assert_eq!(dashboard.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn faults_declare_nothing() {
    let key_fault = FakeReads {
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let key_record = McpAuth {
        object_permission_id: Some(String::from("op-key")),
        ..McpAuth::default()
    };
    assert!(!key_or_team_declares_toolsets(&key_fault, Some(&key_record)).await);

    let team_fault = FakeReads {
        team: Scripted::Fault,
        ..FakeReads::standard()
    };
    let team_record = McpAuth {
        team_id: Some(String::from("t1")),
        ..McpAuth::default()
    };
    assert!(!key_or_team_declares_toolsets(&team_fault, Some(&team_record)).await);
    assert_eq!(
        team_fault.calls(),
        expected(&["has_database", "team_permission:t1:false"])
    );
}
