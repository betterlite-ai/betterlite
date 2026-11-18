use betterlite_error::proxy_error_types::ProxyErrorTypes;

use crate::auth_resource_grants::search::{
    KeySearchGrant, SearchToolFailure, SearchToolObject, TeamSearchGrant, authorize_search_tool,
    can_key_call_search_tool, can_object_call_search_tool, can_team_call_search_tool,
    can_user_view_search_tool, search_tool_names,
};
use crate::auth_resource_grants::tests::support::{
    GrantAnswer, ScriptedGrant, permission, team, user_key,
};

const RESTRICTED_KEY: &str = r#"{"token":"key-1","object_permission":{"object_permission_id":"perm-key","search_tools":["allowed-tool"]}}"#;

const RESTRICTED_TEAM: &str = r#"{"team_id":"team-1","object_permission":{"object_permission_id":"perm-team","search_tools":["allowed-tool"]}}"#;

#[test]
fn absent_permission_unrestricted() {
    let key = user_key(r#"{"token":"key-1"}"#);
    assert!(can_key_call_search_tool("duckduckgo", &key).is_ok());
    assert!(search_tool_names(None).is_empty());
}

#[test]
fn empty_search_list_unrestricted() {
    let empty = user_key(
        r#"{"token":"key-1","object_permission":{"object_permission_id":"perm-1","search_tools":[]}}"#,
    );
    assert!(can_key_call_search_tool("anything", &empty).is_ok());
    let null = user_key(
        r#"{"token":"key-2","object_permission":{"object_permission_id":"perm-2","search_tools":null}}"#,
    );
    assert!(can_key_call_search_tool("anything", &null).is_ok());
    assert!(
        search_tool_names(Some(&permission(
            r#"{"object_permission_id":"perm-3","search_tools":[]}"#
        )))
        .is_empty()
    );
    assert!(
        search_tool_names(Some(&permission(
            r#"{"object_permission_id":"perm-4","search_tools":null}"#
        )))
        .is_empty()
    );
}

#[test]
fn exact_search_name() {
    let key = user_key(RESTRICTED_KEY);
    assert!(can_key_call_search_tool("allowed-tool", &key).is_ok());
    assert!(can_key_call_search_tool("allowed", &key).is_err());
    assert!(can_key_call_search_tool("allowed-tool-2", &key).is_err());
}

#[test]
fn search_no_wildcards() {
    let key = user_key(
        r#"{"token":"key-1","object_permission":{"object_permission_id":"perm-1","search_tools":["tool-*","*"]}}"#,
    );
    let error = can_key_call_search_tool("tool-1", &key).expect_err("a wildcard grants nothing");
    assert_eq!(
        error.error_type,
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    assert_eq!(error.param.as_deref(), Some("search_tool_name"));
    assert_eq!(error.code, "403");
    assert!(error.message.contains("tool-1"), "{}", error.message);
    assert!(error.message.contains("tool-*"), "{}", error.message);
    assert!(can_key_call_search_tool("*", &key).is_ok());
}

#[test]
fn team_missing_allows() {
    assert!(can_team_call_search_tool("tool", None).is_ok());
    let plain = team(r#"{"team_id":"team-1"}"#);
    assert!(can_team_call_search_tool("tool", Some(&plain)).is_ok());
}

#[test]
fn team_denial_uses_key_type() {
    let restricted = team(RESTRICTED_TEAM);
    let denial =
        can_team_call_search_tool("other", Some(&restricted)).expect_err("the team denies");
    assert_eq!(
        denial.error_type,
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    assert_eq!(denial.code, "403");
    assert!(
        denial
            .message
            .starts_with("Team not allowed to access search tool: other."),
        "{}",
        denial.message
    );
    assert_eq!(
        denial.message,
        "Team not allowed to access search tool: other. Allowed search tools: allowed-tool"
    );
    let key = user_key(RESTRICTED_KEY);
    let key_denial = can_key_call_search_tool("other", &key).expect_err("the key denies");
    assert!(
        key_denial
            .message
            .starts_with("Key not allowed to access search tool: other."),
        "{}",
        key_denial.message
    );
    assert_eq!(
        key_denial.message,
        "Key not allowed to access search tool: other. Allowed search tools: allowed-tool"
    );
}

#[test]
fn project_denial_names_the_project() {
    let denial =
        can_object_call_search_tool("other", &["allowed".to_owned()], SearchToolObject::Project)
            .expect_err("the project denies");
    assert!(
        denial
            .message
            .starts_with("Project not allowed to access search tool: other."),
        "{}",
        denial.message
    );
    assert_eq!(
        denial.message,
        "Project not allowed to access search tool: other. Allowed search tools: allowed"
    );
}

#[test]
fn view_key_before_team() {
    let key = ScriptedGrant::new(GrantAnswer::Deny);
    let team = ScriptedGrant::new(GrantAnswer::Allow);
    let visible =
        can_user_view_search_tool("tool", &key, &team).expect("a denial is not a failure");
    assert!(!visible);
    assert_eq!(key.calls(), vec!["tool".to_owned()]);
    assert!(team.calls().is_empty());
}

#[test]
fn view_proxy_error_false() {
    let allowing = ScriptedGrant::new(GrantAnswer::Allow);
    let denying = ScriptedGrant::new(GrantAnswer::Deny);
    assert!(!can_user_view_search_tool("tool", &allowing, &denying).expect("the team denial"));
    assert!(!can_user_view_search_tool("tool", &denying, &allowing).expect("the key denial"));
}

#[test]
fn view_other_error_propagates() {
    let failing = ScriptedGrant::new(GrantAnswer::Fail);
    let allowing = ScriptedGrant::new(GrantAnswer::Allow);
    match can_user_view_search_tool("tool", &failing, &allowing) {
        Err(SearchToolFailure::Lookup { message }) => {
            assert_eq!(message, "the scripted lookup failure");
        }
        other => panic!("the lookup failure propagates, got {other:?}"),
    }
}

#[test]
fn view_trait_grants_drive_the_view() {
    let restricted = user_key(RESTRICTED_KEY);
    let plain_team = team(r#"{"team_id":"team-1"}"#);
    let key = KeySearchGrant { token: &restricted };
    let team = TeamSearchGrant {
        team: Some(&plain_team),
    };
    assert!(can_user_view_search_tool("allowed-tool", &key, &team).expect("the view"));
    assert!(!can_user_view_search_tool("other", &key, &team).expect("the view"));

    let unrestricted = user_key(r#"{"token":"key-2","team_id":"team-1"}"#);
    let restricted_team = team(RESTRICTED_TEAM);
    let open_key = KeySearchGrant {
        token: &unrestricted,
    };
    let closed_team = TeamSearchGrant {
        team: Some(&restricted_team),
    };
    assert!(can_user_view_search_tool("allowed-tool", &open_key, &closed_team).expect("the view"));
    assert!(!can_user_view_search_tool("other", &open_key, &closed_team).expect("the team denial"));
}

#[test]
fn admin_has_no_helper_bypass() {
    let admin = user_key(
        r#"{"token":"key-admin","user_role":"proxy_admin","object_permission":{"object_permission_id":"perm-admin","search_tools":["allowed-tool"]}}"#,
    );
    assert!(can_key_call_search_tool("other", &admin).is_err());
    let viewer = user_key(
        r#"{"token":"key-viewer","user_role":"proxy_admin_viewer","object_permission":{"object_permission_id":"perm-viewer","search_tools":["allowed-tool"]}}"#,
    );
    assert!(can_key_call_search_tool("other", &viewer).is_err());
    assert!(can_key_call_search_tool("allowed-tool", &admin).is_ok());
}

#[test]
fn authorize_denies_on_the_key_before_the_team() {
    let key = user_key(
        r#"{"token":"key-1","team_id":"team-1","object_permission":{"object_permission_id":"perm-key","search_tools":["allowed-tool"]}}"#,
    );
    let allowing_team = team(r#"{"team_id":"team-1"}"#);
    let denial = authorize_search_tool("other", &key, Some(&allowing_team))
        .expect_err("the key denies before the team");
    assert!(
        denial
            .message
            .starts_with("Key not allowed to access search tool: other."),
        "{}",
        denial.message
    );
}

#[test]
fn team_check_runs_only_with_a_team_id() {
    let restricted_team = team(RESTRICTED_TEAM);
    let solo = user_key(
        r#"{"token":"key-solo","object_permission":{"object_permission_id":"perm-key","search_tools":["allowed-tool"]}}"#,
    );
    assert!(authorize_search_tool("allowed-tool", &solo, Some(&restricted_team)).is_ok());
    let member = user_key(
        r#"{"token":"key-member","team_id":"team-1","object_permission":{"object_permission_id":"perm-key","search_tools":["allowed-tool"]}}"#,
    );
    let denial = authorize_search_tool("allowed-tool", &member, Some(&restricted_team))
        .expect_err("the joined key answers to its team");
    assert!(
        denial
            .message
            .starts_with("Team not allowed to access search tool: allowed-tool."),
        "{}",
        denial.message
    );
    assert!(authorize_search_tool("allowed-tool", &member, None).is_ok());
}
