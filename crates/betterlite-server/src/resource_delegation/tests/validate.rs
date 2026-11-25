use betterlite_auth::auth_model_patterns::access_groups::AccessGroupLookup;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_mcp::registry::servers::SharedRegistry;

use crate::resource_delegation::ObjectPermissionGrants;
use crate::resource_delegation::server_validation::{KeyServersError, validate_key_servers};
use crate::resource_delegation::team_read::team_allowed_server_ids;

use super::fixtures::fakes::{FakeGroups, FakeRows, PortFailure};
use super::fixtures::logs::Logs;
use super::fixtures::{
    ceiling_grants, denied_detail, json, reads, registry, registry_server, server_grants,
    string_set, team_fields,
};

struct Case<'case> {
    shared: &'case SharedRegistry,
    rows: &'case FakeRows,
    unified: Option<&'case dyn AccessGroupLookup>,
    admin: bool,
}

async fn check(
    permission: Option<&mut JsonData>,
    team: Option<&TeamFields>,
    case: &Case<'_>,
    existing: Option<&ObjectPermissionGrants>,
) -> Result<(), KeyServersError<PortFailure>> {
    let grant_reads = reads(case.shared, None, case.rows, case.unified);
    validate_key_servers(permission, team, grant_reads, case.admin, existing).await
}

#[tokio::test]
async fn empty_grants_no_io() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let unified = FakeGroups::answering(&[("group-a", &["cfg-u"])]);
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: Some(&unified),
        admin: false,
    };
    let team = team_fields(
        "team-1",
        &["group-a"],
        Some(server_grants(Some(&["srv-a"]), Some(&["group-a"]), None)),
    );
    let mut permission = json("{}");
    check(Some(&mut permission), Some(&team), &case, None)
        .await
        .unwrap();
    check(None, Some(&team), &case, None).await.unwrap();
    assert_eq!(rows.call_count(), 0_usize);
    assert_eq!(unified.call_count(), 0_usize);
    assert_eq!(permission, json("{}"));
}

#[tokio::test]
async fn wrong_shaped_request_does_no_io() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let wrong_shapes =
        json(r#"{"mcp_servers": "srv-a", "mcp_access_groups": {}, "mcp_toolsets": 7}"#);
    let mut request = wrong_shapes.clone();
    check(Some(&mut request), None, &case, None).await.unwrap();
    assert_eq!(request, wrong_shapes);
    assert_eq!(rows.call_count(), 0_usize);
}

#[tokio::test]
async fn server_group_toolset_error_order() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-x", "name": "cfg-x", "transport": "http"}"#,
        )],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let mut all_three =
        json(r#"{"mcp_servers": ["cfg-x"], "mcp_access_groups": ["g"], "mcp_toolsets": ["t"]}"#);
    let server_error = check(Some(&mut all_three), None, &case, None)
        .await
        .unwrap_err();
    assert!(denied_detail(server_error).contains("Only globally available"));
    let mut groups_and_toolsets = json(r#"{"mcp_access_groups": ["g"], "mcp_toolsets": ["t"]}"#);
    let group_error = check(Some(&mut groups_and_toolsets), None, &case, None)
        .await
        .unwrap_err();
    assert!(denied_detail(group_error).contains("MCP access groups cannot be assigned"));
    let mut toolsets_only = json(r#"{"mcp_toolsets": ["t"]}"#);
    let toolset_error = check(Some(&mut toolsets_only), None, &case, None)
        .await
        .unwrap_err();
    assert!(denied_detail(toolset_error).contains("MCP toolsets cannot be assigned"));
}

#[tokio::test]
async fn stale_servers_fall_through_to_group_error() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let mut stale_server =
        json(r#"{"mcp_servers": ["gone"], "mcp_access_groups": ["g"], "mcp_toolsets": ["t"]}"#);
    let error = check(Some(&mut stale_server), None, &case, None)
        .await
        .unwrap_err();
    assert!(denied_detail(error).contains("MCP access groups cannot be assigned"));
}

#[tokio::test]
async fn personal_vs_team_admin() {
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "cfg-a", "name": "cfg-a", "transport": "http"}"#),
            registry_server(
                r#"{"server_id": "cfg-global", "name": "cfg-global", "transport": "http", "allow_all_keys": true}"#,
            ),
        ],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let admin_case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: true,
    };
    let mut personal_admin = json(r#"{"mcp_servers": ["cfg-a"], "mcp_access_groups": ["g-any"]}"#);
    check(Some(&mut personal_admin), None, &admin_case, None)
        .await
        .unwrap();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let mut personal = json(r#"{"mcp_servers": ["cfg-a"]}"#);
    let error = check(Some(&mut personal), None, &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key is not in a team. Only globally available (allow_all_keys) MCP servers can be assigned: ['cfg-global']. Disallowed servers: ['cfg-a']."
    );
    let mut global = json(r#"{"mcp_servers": ["cfg-global"]}"#);
    check(Some(&mut global), None, &case, None).await.unwrap();
}

#[tokio::test]
async fn team_ceiling_applies_to_team_admin() {
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "cfg-a", "name": "cfg-a", "transport": "http"}"#),
            registry_server(r#"{"server_id": "cfg-b", "name": "cfg-b", "transport": "http"}"#),
            registry_server(
                r#"{"server_id": "cfg-global", "name": "cfg-global", "transport": "http", "allow_all_keys": true}"#,
            ),
        ],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let team = team_fields(
        "team-1",
        &[],
        Some(server_grants(Some(&["cfg-b"]), Some(&["g-team"]), None)),
    );
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: true,
    };
    let mut member = json(r#"{"mcp_servers": ["cfg-b"], "mcp_access_groups": ["g-team"]}"#);
    check(Some(&mut member), Some(&team), &case, None)
        .await
        .unwrap();
    let mut server = json(r#"{"mcp_servers": ["cfg-a"]}"#);
    let error = check(Some(&mut server), Some(&team), &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key requests MCP servers not allowed by team 'team-1': ['cfg-a']. Team allows: ['cfg-b']. Global (allow_all_keys) servers: ['cfg-global']."
    );
}

#[tokio::test]
async fn team_group_ceiling_applies_to_team_admin() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let team = team_fields(
        "team-1",
        &[],
        Some(server_grants(None, Some(&["g-team"]), None)),
    );
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: true,
    };
    let mut group = json(r#"{"mcp_access_groups": ["g-any"]}"#);
    let error = check(Some(&mut group), Some(&team), &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key requests MCP access groups not allowed by team 'team-1': ['g-any']. Team allows: ['g-team']."
    );
}

#[tokio::test]
async fn group_source_not_unified() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-u", "name": "cfg-u", "transport": "http", "access_groups": ["group-a"]}"#,
        )],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let unified = FakeGroups::answering(&[("group-a", &["cfg-u"])]);
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: Some(&unified),
        admin: false,
    };
    let team = team_fields(
        "team-1",
        &["group-a"],
        Some(server_grants(None, None, None)),
    );
    let mut unified_group = json(r#"{"mcp_access_groups": ["group-a"]}"#);
    let error = check(Some(&mut unified_group), Some(&team), &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key requests MCP access groups not allowed by team 'team-1': ['group-a']. Team allows: []."
    );
    let mut unified_server = json(r#"{"mcp_servers": ["cfg-u"]}"#);
    check(Some(&mut unified_server), Some(&team), &case, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_object_permission_group_allows() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let team = team_fields(
        "team-1",
        &[],
        Some(server_grants(None, Some(&["group-a"]), None)),
    );
    let mut group = json(r#"{"mcp_access_groups": ["group-a"]}"#);
    check(Some(&mut group), Some(&team), &case, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn group_not_grandfathered() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let existing = server_grants(None, Some(&["old-group"]), None);
    let team = team_fields("team-1", &[], Some(server_grants(None, None, None)));
    let mut kept_group = json(r#"{"mcp_access_groups": ["old-group"]}"#);
    let team_error = validate_key_servers(
        Some(&mut kept_group),
        Some(&team),
        reads(&shared, None, &rows, None),
        false,
        Some(&existing),
    )
    .await
    .unwrap_err();
    assert_eq!(
        denied_detail(team_error),
        "Key requests MCP access groups not allowed by team 'team-1': ['old-group']. Team allows: []."
    );
    let mut personal_group = json(r#"{"mcp_access_groups": ["old-group"]}"#);
    let personal_error = validate_key_servers(
        Some(&mut personal_group),
        None,
        reads(&shared, None, &rows, None),
        false,
        Some(&existing),
    )
    .await
    .unwrap_err();
    assert_eq!(
        denied_detail(personal_error),
        "Key is not in a team. MCP access groups cannot be assigned to keys outside of a team. Disallowed groups: ['old-group']."
    );
}

#[tokio::test]
async fn toolset_empty_ceiling_vs_subset() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let requested = r#"{"mcp_toolsets": ["t-a", "t-b"]}"#;
    let absent = team_fields("team-1", &[], Some(ceiling_grants(None, None)));
    let mut absent_request = json(requested);
    check(Some(&mut absent_request), Some(&absent), &case, None)
        .await
        .unwrap();
    let empty = team_fields("team-1", &[], Some(ceiling_grants(Some(&[]), None)));
    let mut empty_request = json(requested);
    check(Some(&mut empty_request), Some(&empty), &case, None)
        .await
        .unwrap();
    let open = team_fields("team-1", &[], None);
    check(Some(&mut json(requested)), Some(&open), &case, None)
        .await
        .unwrap();
    let ceiling = team_fields("team-1", &[], Some(ceiling_grants(Some(&["t-a"]), None)));
    let mut subset = json(r#"{"mcp_toolsets": ["t-a"]}"#);
    check(Some(&mut subset), Some(&ceiling), &case, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn toolset_ceiling_denies_members_and_admins() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let ceiling = team_fields("team-1", &[], Some(ceiling_grants(Some(&["t-a"]), None)));
    let requested = r#"{"mcp_toolsets": ["t-a", "t-b"]}"#;
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let mut outside = json(requested);
    let member_error = check(Some(&mut outside), Some(&ceiling), &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(member_error),
        "Key requests MCP toolsets not allowed by team 'team-1': ['t-b']. Team allows: ['t-a']."
    );
    let admin_case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: true,
    };
    let mut outside_admin = json(requested);
    let admin_error = check(Some(&mut outside_admin), Some(&ceiling), &admin_case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(admin_error),
        "Key requests MCP toolsets not allowed by team 'team-1': ['t-b']. Team allows: ['t-a']."
    );
}

#[tokio::test]
async fn toolset_personal_admin_vs_non_admin() {
    let shared = registry(Vec::new(), Vec::new());
    let rows = FakeRows::silent();
    let requested = r#"{"mcp_toolsets": ["t-a", "t-b"]}"#;
    let admin_case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: true,
    };
    let mut personal_admin = json(requested);
    check(Some(&mut personal_admin), None, &admin_case, None)
        .await
        .unwrap();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let mut personal = json(requested);
    let error = check(Some(&mut personal), None, &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key is not in a team. MCP toolsets cannot be assigned to personal keys by non-admin callers. Disallowed toolsets: ['t-a', 't-b']."
    );
}

#[tokio::test]
async fn unified_and_direct_team_grants() {
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "cfg-b", "name": "cfg-b", "transport": "http"}"#),
            registry_server(r#"{"server_id": "cfg-c", "name": "cfg-c", "transport": "http"}"#),
            registry_server(r#"{"server_id": "cfg-u", "name": "cfg-u", "transport": "http"}"#),
        ],
        Vec::new(),
    );
    let rows = FakeRows::answering(&["db-group-server"]);
    let unified = FakeGroups::answering(&[("group-a", &["cfg-u", "ghost-name"])]);
    let team = team_fields(
        "team-1",
        &["group-a"],
        Some(server_grants(
            Some(&["cfg-b"]),
            Some(&["group-b"]),
            Some(json(r#"{"cfg-c": []}"#)),
        )),
    );
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: Some(&unified),
        admin: false,
    };
    let allowed =
        team_allowed_server_ids(Some(&team), &reads(&shared, None, &rows, Some(&unified)))
            .await
            .unwrap();
    assert_eq!(
        allowed,
        string_set(&["cfg-b", "cfg-c", "cfg-u", "db-group-server", "ghost-name"])
    );
    let mut granted = json(r#"{"mcp_servers": ["cfg-b", "cfg-c", "cfg-u", "db-group-server"]}"#);
    check(Some(&mut granted), Some(&team), &case, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_permission_absent_returns_unified_ids() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-u", "name": "cfg-u", "transport": "http"}"#,
        )],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let unified = FakeGroups::answering(&[("group-a", &["cfg-u", "ghost-name"])]);
    let team = team_fields("team-1", &["group-a"], None);
    let grant_reads = reads(&shared, None, &rows, Some(&unified));
    let allowed = team_allowed_server_ids(Some(&team), &grant_reads)
        .await
        .unwrap();
    assert_eq!(allowed, string_set(&["cfg-u", "ghost-name"]));
}

#[tokio::test]
async fn unlisted_server_denial_lists_all_team_grants() {
    let shared = registry(
        vec![registry_server(
            r#"{"server_id": "cfg-a", "name": "cfg-a", "transport": "http"}"#,
        )],
        Vec::new(),
    );
    let rows = FakeRows::answering(&["db-group-server"]);
    let unified = FakeGroups::answering(&[("group-a", &["cfg-u", "ghost-name"])]);
    let team = team_fields(
        "team-1",
        &["group-a"],
        Some(server_grants(
            Some(&["cfg-b"]),
            Some(&["group-b"]),
            Some(json(r#"{"cfg-c": []}"#)),
        )),
    );
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: Some(&unified),
        admin: false,
    };
    let mut unlisted = json(r#"{"mcp_servers": ["cfg-a"]}"#);
    let error = check(Some(&mut unlisted), Some(&team), &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key requests MCP servers not allowed by team 'team-1': ['cfg-a']. Team allows: ['cfg-b', 'cfg-c', 'cfg-u', 'db-group-server', 'ghost-name']. Global (allow_all_keys) servers: []."
    );
}

#[tokio::test]
async fn mcp_denial_messages() {
    let shared = registry(
        vec![
            registry_server(
                r#"{"server_id": "srv-a", "name": "A", "alias": "shared-alias", "transport": "http"}"#,
            ),
            registry_server(
                r#"{"server_id": "srv-b", "name": "B", "alias": "shared-alias", "transport": "http"}"#,
            ),
            registry_server(
                r#"{"server_id": "srv-global", "name": "global", "transport": "http", "allow_all_keys": true}"#,
            ),
        ],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let team = team_fields(
        "team-1",
        &[],
        Some(server_grants(Some(&["srv-a"]), None, None)),
    );
    let mut shared_alias = json(r#"{"mcp_servers": ["shared-alias"]}"#);
    let error = check(Some(&mut shared_alias), Some(&team), &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key requests MCP servers not allowed by team 'team-1': ['srv-b']. Team allows: ['srv-a']. Global (allow_all_keys) servers: ['srv-global']."
    );
    let mut global = json(r#"{"mcp_servers": ["srv-global"]}"#);
    check(Some(&mut global), Some(&team), &case, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn personal_server_denial_lists_globals() {
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "srv-b", "name": "B", "transport": "http"}"#),
            registry_server(
                r#"{"server_id": "srv-global", "name": "global", "transport": "http", "allow_all_keys": true}"#,
            ),
        ],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let case = Case {
        shared: &shared,
        rows: &rows,
        unified: None,
        admin: false,
    };
    let mut personal = json(r#"{"mcp_servers": ["srv-b"]}"#);
    let error = check(Some(&mut personal), None, &case, None)
        .await
        .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key is not in a team. Only globally available (allow_all_keys) MCP servers can be assigned: ['srv-global']. Disallowed servers: ['srv-b']."
    );
}

#[tokio::test]
async fn stale_warning_mutates_before_denial() {
    let (logs, _guard) = Logs::capture();
    let shared = registry(
        vec![
            registry_server(r#"{"server_id": "cfg-x", "name": "cfg-x", "transport": "http"}"#),
            registry_server(r#"{"server_id": "cfg-y", "name": "cfg-y", "transport": "http"}"#),
        ],
        Vec::new(),
    );
    let rows = FakeRows::silent();
    let mut permission = json(
        r#"{"mcp_servers": ["gone-b", "cfg-y", "gone-a", "cfg-y", "cfg-x"], "mcp_tool_permissions": {"gone-b": ["t"], "cfg-x": ["t1", "t1"]}}"#,
    );
    let error = validate_key_servers(
        Some(&mut permission),
        None,
        reads(&shared, None, &rows, None),
        false,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        denied_detail(error),
        "Key is not in a team. Only globally available (allow_all_keys) MCP servers can be assigned: []. Disallowed servers: ['cfg-x', 'cfg-y']."
    );
    assert_eq!(
        permission,
        json(r#"{"mcp_servers": ["cfg-y", "cfg-x"], "mcp_tool_permissions": {"cfg-x": ["t1"]}}"#)
    );
    let captured = logs.text();
    assert!(captured.contains(
        "validate_key_mcp_servers_against_team: ignoring stale MCP server identifiers (no longer in registry or DB): ['gone-a', 'gone-b']"
    ));
    assert_eq!(
        captured
            .matches("ignoring stale MCP server identifiers")
            .count(),
        1_usize
    );
}
