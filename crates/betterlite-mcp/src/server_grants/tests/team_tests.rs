use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::PermissionList;
use rstest::rstest;

use super::super::team::team_id_for_grant;
use super::{
    FakeReads, Scripted, four_source_row, grant_auth, grants, names, row, sorted, team_record,
    toolset_grants,
};

#[rstest]
#[case::own_team(Some("team-1"), Some("team-1"))]
#[case::no_team(None, None)]
#[case::empty_team(Some(""), None)]
#[case::dashboard_team(Some("litellm-dashboard"), None)]
fn team_id_is_only_the_callers_own(#[case] team_id: Option<&str>, #[case] expected: Option<&str>) {
    let auth = grant_auth(team_id, false);
    assert_eq!(team_id_for_grant(Some(&auth)), expected);
    assert_eq!(team_id_for_grant(None), None);
}

#[rstest]
#[case::no_auth(None)]
#[case::no_team(Some(None))]
#[case::dashboard_team(Some(Some("litellm-dashboard")))]
#[tokio::test]
async fn team_without_a_grant_team_reads_nothing(#[case] team: Option<Option<&str>>) {
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(Some(four_source_row())))),
        user_teams: Scripted::Value(Some(names(&["team-a", "team-b"]))),
        ..FakeReads::standard()
    });
    let auth = team.map(|team_id| grant_auth(team_id, false));
    assert!(
        server_grants
            .team_servers(auth.as_ref())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(server_grants.reads.calls().is_empty());
}

#[rstest]
#[case::empty_id("")]
#[case::dashboard_id("litellm-dashboard")]
#[tokio::test]
async fn team_single_team_skips_the_empty_and_dashboard_ids(#[case] team_id: &str) {
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(None))),
        unified: Scripted::Value(names(&["srv-group"])),
        ..FakeReads::standard()
    });
    assert!(
        server_grants
            .single_team_servers(team_id, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(server_grants.reads.calls().is_empty());
}

#[tokio::test]
async fn team_without_a_database_is_empty() {
    let server_grants = grants(FakeReads {
        database: false,
        team: Scripted::Value(Some(team_record(None))),
        unified: Scripted::Value(names(&["srv-group"])),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), false);
    assert!(
        server_grants
            .team_servers(Some(&auth))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(server_grants.reads.calls().is_empty());
}

#[tokio::test]
async fn team_missing_row_is_empty() {
    let server_grants = grants(FakeReads::standard());
    let auth = grant_auth(Some("team-1"), false);
    assert!(
        server_grants
            .team_servers(Some(&auth))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(server_grants.reads.calls(), names(&["team:team-1:false"]));
}

#[tokio::test]
async fn team_blocked_grants_nothing() {
    let mut team = team_record(Some(four_source_row()));
    team.blocked = true;
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team)),
        unified: Scripted::Value(names(&["srv-group"])),
        legacy: Scripted::Value(names(&["srv-legacy"])),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), false);
    assert!(
        server_grants
            .team_servers(Some(&auth))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(server_grants.reads.calls(), names(&["team:team-1:false"]));
}

#[tokio::test]
async fn team_all_proxy_servers_is_exactly_the_registry() {
    let mut permission = four_source_row();
    permission.mcp_servers = PermissionList(Some(names(&["srv-direct", "all-proxy-mcpservers"])));
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(Some(permission)))),
        unified: Scripted::Value(names(&["srv-outside"])),
        legacy: Scripted::Value(names(&["srv-legacy"])),
        toolsets: Scripted::Value(toolset_grants()),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), false);
    assert_eq!(
        server_grants.team_servers(Some(&auth)).await.unwrap(),
        names(&["srv-gh", "srv-direct", "srv-group", "srv-tools"])
    );
    assert_eq!(
        server_grants.reads.calls(),
        names(&[
            "team:team-1:false",
            r#"access_group_server_ids:["ag-team"]:false"#,
            "registry_server_ids",
        ])
    );
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn team_union_of_five_sources_is_distinct(#[case] fresh: bool) {
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(Some(four_source_row())))),
        unified: Scripted::Value(names(&["srv-team-group", "srv-direct"])),
        legacy: Scripted::Value(names(&["srv-group", "srv-direct"])),
        toolsets: Scripted::Value(toolset_grants()),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), fresh);
    let servers = server_grants.team_servers(Some(&auth)).await.unwrap();
    assert_eq!(
        sorted(servers),
        names(&[
            "srv-direct",
            "srv-gh",
            "srv-group",
            "srv-team-group",
            "srv-tools",
            "srv-toolset",
            "unknown-name",
        ])
    );
    assert_eq!(
        server_grants.reads.calls(),
        Vec::from([
            format!("team:team-1:{fresh}"),
            format!(r#"access_group_server_ids:["ag-team"]:{fresh}"#),
            format!(r#"servers_from_access_groups:["grp-1"]:{fresh}"#),
            format!(r#"resolve_toolset_grants:["ts-1"]:{fresh}"#),
            String::from(r#"expand_permission_list:["github", "srv-direct", "unknown-name"]"#),
            String::from(r#"expand_tool_permissions:Some({"tool-alias": ["read"]})"#),
        ])
    );
}

#[tokio::test]
async fn team_without_a_permission_gets_only_its_access_group_servers() {
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(None))),
        unified: Scripted::Value(names(&["srv-team-group", "srv-x", "srv-team-group"])),
        legacy: Scripted::Value(names(&["srv-legacy"])),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), false);
    assert_eq!(
        server_grants.team_servers(Some(&auth)).await.unwrap(),
        names(&["srv-team-group", "srv-x"])
    );
}

#[tokio::test]
async fn team_permission_without_toolsets_is_not_unloadable() {
    let mut permission = row();
    permission.mcp_tool_permissions = Some(BTreeMap::from([(
        String::from("srv-tools"),
        names(&["read"]),
    )]));
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(Some(permission)))),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), false);
    assert_eq!(
        server_grants.team_servers(Some(&auth)).await.unwrap(),
        names(&["srv-tools"])
    );
}
