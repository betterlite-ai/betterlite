use std::error::Error as _;

use super::super::error::{EntitlementError, UnloadableEntitlementError};
use super::super::hydration::load_named_object_permission;
use super::super::principal::{
    NamedPrincipal, key_object_permission_hydrated, named_object_permission, team_object_permission,
};
use super::super::reads::{EndUserEntitlement, ObjectPermission, OrgPermissionLink};
use super::super::record::McpAuth;
use super::{FakeReads, Scripted, StoreDown, auth_from_json, expected, mcp_auth, row};

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
        ..mcp_auth()
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
        ..mcp_auth()
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
        ..mcp_auth()
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
        ..mcp_auth()
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
        ..mcp_auth()
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
        ..mcp_auth()
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
        ..mcp_auth()
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
        named_object_permission(&reads, &McpAuth::default(), NamedPrincipal::Org)
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let auth = McpAuth {
        org_id: Some(String::from("o1")),
        ..mcp_auth()
    };
    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    assert!(
        named_object_permission(&offline, &auth, NamedPrincipal::Org)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));

    let unnamed = FakeReads::standard();
    assert!(
        named_object_permission(&unnamed, &auth, NamedPrincipal::Org)
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
        ..mcp_auth()
    };
    assert_eq!(
        named_object_permission(&reads, &auth, NamedPrincipal::Org)
            .await
            .unwrap(),
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
        ..mcp_auth()
    };
    let absent = FakeReads {
        org: Scripted::Value(OrgPermissionLink::Named(String::from("op-org"))),
        ..FakeReads::standard()
    };
    let error = named_object_permission(&absent, &auth, NamedPrincipal::Org)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Unloadable(_)));
    assert!(error.to_string().contains("op-org"));

    let unreadable = FakeReads {
        org: Scripted::Value(OrgPermissionLink::Named(String::from("op-org"))),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let fault = named_object_permission(&unreadable, &auth, NamedPrincipal::Org)
        .await
        .unwrap_err();
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
        ..mcp_auth()
    };
    let error = named_object_permission(&reads, &auth, NamedPrincipal::Org)
        .await
        .unwrap_err();
    assert!(matches!(&error, EntitlementError::Read { .. }));
}

#[tokio::test]
async fn end_user_permission_is_none_without_an_id_a_row_or_a_named_permission() {
    let reads = FakeReads::standard();
    assert!(
        named_object_permission(&reads, &McpAuth::default(), NamedPrincipal::EndUser)
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let auth = McpAuth {
        end_user_id: Some(String::from("eu")),
        ..mcp_auth()
    };
    let absent = FakeReads::standard();
    assert!(
        named_object_permission(&absent, &auth, NamedPrincipal::EndUser)
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
        named_object_permission(&unnamed, &auth, NamedPrincipal::EndUser)
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
        ..mcp_auth()
    };
    assert!(
        named_object_permission(&reads, &auth, NamedPrincipal::EndUser)
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
        ..mcp_auth()
    };
    assert_eq!(
        named_object_permission(&reads, &auth, NamedPrincipal::EndUser)
            .await
            .unwrap(),
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
        ..mcp_auth()
    };
    assert_eq!(
        named_object_permission(&reads, &auth, NamedPrincipal::EndUser)
            .await
            .unwrap(),
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
        ..mcp_auth()
    };
    let absent = FakeReads {
        end_user: Scripted::Value(Some(EndUserEntitlement {
            object_permission: None,
            object_permission_id: Some(String::from("op-named")),
        })),
        ..FakeReads::standard()
    };
    let error = named_object_permission(&absent, &auth, NamedPrincipal::EndUser)
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
    let fault = named_object_permission(&unreadable, &auth, NamedPrincipal::EndUser)
        .await
        .unwrap_err();
    assert!(fault.source().is_some());
}
