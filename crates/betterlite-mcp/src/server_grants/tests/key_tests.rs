use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, PermissionList};
use rstest::rstest;

use super::super::auth::GrantAuth;
use super::{
    FakeReads, Scripted, four_source_row, grant_auth, grants, key_auth, names, row, sorted,
    toolset_grants,
};

fn union_reads() -> FakeReads {
    FakeReads {
        legacy: Scripted::Value(names(&["srv-group", "srv-direct"])),
        toolsets: Scripted::Value(toolset_grants()),
        ..FakeReads::standard()
    }
}

#[tokio::test]
async fn key_without_auth_reads_nothing() {
    let server_grants = grants(union_reads());
    assert!(server_grants.key_servers(None).await.is_empty());
    assert!(
        server_grants
            .key_access_group_servers(None)
            .await
            .is_empty()
    );
    assert!(server_grants.reads.calls().is_empty());
}

#[tokio::test]
async fn key_without_a_permission_is_empty() {
    let server_grants = grants(union_reads());
    let auth = grant_auth(None, false);
    assert!(server_grants.key_servers(Some(&auth)).await.is_empty());
    assert!(server_grants.reads.calls().is_empty());
}

#[rstest]
#[case::alone(&["no-mcp-servers"])]
#[case::with_other_servers(&["github", "no-mcp-servers", "srv-direct"])]
#[tokio::test]
async fn key_no_mcp_servers_returns_the_literal_alone(#[case] mcp_servers: &[&str]) {
    let server_grants = grants(union_reads());
    let mut permission = four_source_row();
    permission.mcp_servers = PermissionList(Some(names(mcp_servers)));
    let auth = key_auth(permission, false);
    assert_eq!(
        server_grants.key_servers(Some(&auth)).await,
        names(&["no-mcp-servers"])
    );
    assert!(server_grants.reads.calls().is_empty());
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn key_union_of_four_sources_is_distinct(#[case] fresh: bool) {
    let server_grants = grants(union_reads());
    let auth = key_auth(four_source_row(), fresh);
    let servers = server_grants.key_servers(Some(&auth)).await;
    assert_eq!(
        sorted(servers),
        names(&[
            "srv-direct",
            "srv-gh",
            "srv-group",
            "srv-tools",
            "srv-toolset",
            "unknown-name"
        ])
    );
    assert_eq!(
        server_grants.reads.calls(),
        Vec::from([
            String::from(r#"expand_permission_list:["github", "srv-direct", "unknown-name"]"#),
            format!(r#"servers_from_access_groups:["grp-1"]:{fresh}"#),
            String::from(r#"expand_tool_permissions:Some({"tool-alias": ["read"]})"#),
            format!(r#"resolve_toolset_grants:["ts-1"]:{fresh}"#),
        ])
    );
}

#[tokio::test]
async fn key_all_proxy_servers_passes_through_as_a_name() {
    let server_grants = grants(FakeReads::standard());
    let mut permission = row();
    permission.mcp_servers = PermissionList(Some(names(&["all-proxy-mcpservers"])));
    let auth = key_auth(permission, false);
    assert_eq!(
        server_grants.key_servers(Some(&auth)).await,
        names(&["all-proxy-mcpservers"])
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(Vec::new()))]
#[tokio::test]
async fn key_without_declared_toolsets_skips_the_resolver(#[case] toolsets: Option<Vec<String>>) {
    let server_grants = grants(union_reads());
    let mut permission = four_source_row();
    permission.mcp_toolsets = toolsets;
    let auth = key_auth(permission, false);
    let servers = server_grants.key_servers(Some(&auth)).await;
    assert!(!servers.contains(&String::from("srv-toolset")));
    assert!(
        server_grants
            .reads
            .calls()
            .iter()
            .all(|call| !call.starts_with("resolve_toolset_grants"))
    );
}

// the team path raises on this same empty answer; a key only adds no toolset servers
#[tokio::test]
async fn key_toolsets_resolving_to_nothing_keep_the_other_sources() {
    let server_grants = grants(FakeReads {
        toolsets: Scripted::Value(BTreeMap::new()),
        ..union_reads()
    });
    let auth = key_auth(four_source_row(), false);
    assert_eq!(
        sorted(server_grants.key_servers(Some(&auth)).await),
        names(&[
            "srv-direct",
            "srv-gh",
            "srv-group",
            "srv-tools",
            "unknown-name"
        ])
    );
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn key_hydrates_a_named_permission_with_the_fresh_flag(#[case] fresh: bool) {
    let mut permission = row();
    permission.mcp_servers = PermissionList(Some(names(&["srv-direct"])));
    let server_grants = grants(FakeReads {
        permission: Scripted::Value(Some(permission)),
        ..FakeReads::standard()
    });
    let mut auth = grant_auth(None, fresh);
    auth.mcp.object_permission_id = Some(String::from("op-key"));
    assert_eq!(
        server_grants.key_servers(Some(&auth)).await,
        names(&["srv-direct"])
    );
    assert_eq!(
        server_grants.reads.calls().first(),
        Some(&format!("permission:op-key:{fresh}"))
    );
}

#[rstest]
#[case::permission_cached(Fault::Permission, false)]
#[case::permission_fresh(Fault::Permission, true)]
#[case::access_groups_cached(Fault::AccessGroups, false)]
#[case::access_groups_fresh(Fault::AccessGroups, true)]
#[case::toolsets_cached(Fault::Toolsets, false)]
#[case::toolsets_fresh(Fault::Toolsets, true)]
#[tokio::test]
async fn key_read_faults_fail_open_to_an_empty_list(#[case] fault: Fault, #[case] fresh: bool) {
    let mut reads = union_reads();
    let mut auth = key_auth(four_source_row(), fresh);
    match fault {
        Fault::Permission => {
            reads.permission = Scripted::Fault;
            auth.mcp.object_permission = None;
            auth.mcp.object_permission_id = Some(String::from("op-key"));
        }
        Fault::AccessGroups => reads.legacy = Scripted::Fault,
        Fault::Toolsets => reads.toolsets = Scripted::Fault,
    }
    let server_grants = grants(reads);
    assert!(server_grants.key_servers(Some(&auth)).await.is_empty());
}

#[derive(Clone, Copy, Debug)]
enum Fault {
    Permission,
    AccessGroups,
    Toolsets,
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn key_access_group_servers_expand_the_unified_ids(#[case] fresh: bool) {
    let server_grants = grants(FakeReads {
        unified: Scripted::Value(names(&["github", "srv-x"])),
        ..FakeReads::standard()
    });
    let mut auth = grant_auth(None, fresh);
    auth.access_group_ids = names(&["ag-1"]);
    assert_eq!(
        sorted(server_grants.key_access_group_servers(Some(&auth)).await),
        names(&["srv-gh", "srv-x"])
    );
    assert_eq!(
        server_grants.reads.calls(),
        Vec::from([
            format!(r#"access_group_server_ids:["ag-1"]:{fresh}"#),
            String::from(r#"expand_permission_list:["github", "srv-x"]"#),
        ])
    );
}

#[tokio::test]
async fn key_access_group_servers_are_empty_when_nothing_resolves() {
    let server_grants = grants(FakeReads::standard());
    let mut auth = grant_auth(None, false);
    auth.access_group_ids = names(&["ag-1"]);
    assert!(
        server_grants
            .key_access_group_servers(Some(&auth))
            .await
            .is_empty()
    );
    assert_eq!(
        server_grants.reads.calls(),
        names(&[r#"access_group_server_ids:["ag-1"]:false"#])
    );
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn key_access_group_servers_are_empty_on_a_read_fault(#[case] fresh: bool) {
    let server_grants = grants(FakeReads {
        unified: Scripted::Fault,
        ..FakeReads::standard()
    });
    let mut auth = grant_auth(None, fresh);
    auth.access_group_ids = names(&["ag-1"]);
    assert!(
        server_grants
            .key_access_group_servers(Some(&auth))
            .await
            .is_empty()
    );
}

#[rstest]
#[case::listed(r#"{"team_id": "t1", "access_group_ids": ["ag-1", "ag-2"]}"#, &["ag-1", "ag-2"])]
#[case::null(r#"{"team_id": "t1", "access_group_ids": null}"#, &[])]
#[case::absent(r#"{"team_id": "t1"}"#, &[])]
fn key_access_groups_come_from_the_auth_record(#[case] json: &str, #[case] expected: &[&str]) {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(json).unwrap();
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    let record = GrantAuth::from_auth(&auth).unwrap();
    assert_eq!(record.access_group_ids, names(expected));
    assert_eq!(record.mcp.team_id.as_deref(), Some("t1"));
}
