use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;

use super::support::{FakeTeams, access, auth, caller, org_admins, row, team};
use crate::memory_records::access::assert_write_access;
use crate::memory_records::caller::{MemoryCaller, is_admin};
use crate::memory_records::filter::{MemoryWhere, visibility_filter};
use crate::python_hooks::test_support::capture;

fn wire(filter: &MemoryWhere) -> String {
    serde_json::to_string(filter).unwrap()
}

#[test]
fn admin_unrestricted_view() {
    let admin = caller(Some("alice"), Some("team-a"), Some(UserRole::ProxyAdmin));
    assert_eq!(visibility_filter(&admin), None);
    let viewer = caller(None, None, Some(UserRole::ProxyAdminViewer));
    assert_eq!(visibility_filter(&viewer), None);
}

#[test]
fn non_admin_user_visibility() {
    let filter = visibility_filter(&caller(Some("alice"), None, None)).unwrap();
    assert_eq!(wire(&filter), r#"{"user_id":"alice"}"#);
}

#[test]
fn non_admin_team_visibility() {
    let filter = visibility_filter(&caller(None, Some("team-a"), None)).unwrap();
    assert_eq!(wire(&filter), r#"{"team_id":"team-a"}"#);
}

#[test]
fn non_admin_or_visibility() {
    let customer = caller(Some("alice"), Some("team-a"), Some(UserRole::Customer));
    let filter = visibility_filter(&customer).unwrap();
    assert_eq!(
        wire(&filter),
        r#"{"OR":[{"user_id":"alice"},{"team_id":"team-a"}]}"#
    );
}

#[test]
fn identity_less_no_match() {
    let filter = visibility_filter(&caller(None, None, None)).unwrap();
    assert_eq!(
        filter,
        MemoryWhere::MemoryId {
            memory_id: "__no_match__".to_owned(),
        }
    );
    assert_eq!(wire(&filter), r#"{"memory_id":"__no_match__"}"#);
}

#[test]
fn caller_projects_the_key_principal() {
    let key = auth(Some("alice"), Some("team-a"), Some("proxy_admin"));
    let projected = MemoryCaller::try_from(&key).unwrap();
    assert_eq!(projected.user_id.as_deref(), Some("alice"));
    assert_eq!(projected.team_id.as_deref(), Some("team-a"));
    assert_eq!(projected.user_role, Some(UserRole::ProxyAdmin));
    assert!(projected.admin_view);
    assert!(is_admin(&projected));
}

#[test]
fn admin_viewer_is_not_a_writer() {
    let key = auth(None, Some("team-a"), Some("proxy_admin_viewer"));
    let projected = MemoryCaller::try_from(&key).unwrap();
    assert_eq!(projected.user_role, Some(UserRole::ProxyAdminViewer));
    assert!(projected.admin_view);
    assert!(!is_admin(&projected));
}

#[tokio::test]
async fn personal_ownership_allowed() {
    let personal_row = row("mem-1", "note", Some("alice"), Some("team-a"));
    let teams = FakeTeams::failing();
    let owner = caller(Some("alice"), None, None);
    assert_write_access(&owner, &personal_row, &teams, &access(Vec::new(), false))
        .await
        .unwrap();
    assert!(teams.lookups().is_empty());

    let teammate = caller(Some("bob"), Some("team-a"), None);
    let error = assert_write_access(&teammate, &personal_row, &teams, &access(Vec::new(), false))
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(
        error.detail,
        "You do not have permission to modify this memory entry."
    );
}

#[tokio::test]
async fn proxy_admin_may_write_any_row() {
    let personal_row = row("mem-1", "note", Some("alice"), Some("team-a"));
    let teams = FakeTeams::failing();
    let admin = caller(Some("root"), None, Some(UserRole::ProxyAdmin));
    assert_write_access(&admin, &personal_row, &teams, &access(Vec::new(), false))
        .await
        .unwrap();
    assert!(teams.lookups().is_empty());
}

#[tokio::test]
async fn team_row_requires_admin() {
    let team_row = row("mem-1", "note", None, Some("team-a"));
    let member_teams = FakeTeams::new(Some(team("team-a", None, &[])));
    let member = caller(Some("bob"), Some("team-a"), None);
    let error = assert_write_access(
        &member,
        &team_row,
        &member_teams,
        &access(Vec::new(), false),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(member_teams.lookups(), vec!["team-a".to_owned()]);

    let admin_teams = FakeTeams::new(Some(team("team-a", None, &["bob"])));
    assert_write_access(&member, &team_row, &admin_teams, &access(Vec::new(), false))
        .await
        .unwrap();
}

#[tokio::test]
async fn team_row_allows_org_admin() {
    let team_row = row("mem-1", "note", None, Some("team-a"));
    let teams = FakeTeams::new(Some(team("team-a", Some("org-1"), &[])));
    let member = caller(Some("bob"), Some("team-a"), None);

    let org_access = access(org_admins(&[("bob", "org-1")]), false);
    assert_write_access(&member, &team_row, &teams, &org_access)
        .await
        .unwrap();

    let other_access = access(org_admins(&[("carol", "org-1")]), false);
    let error = assert_write_access(&member, &team_row, &teams, &other_access)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn team_lookup_failure_denies() {
    let (captured, guard) = capture();
    let team_row = row("mem-1", "note", None, Some("team-a"));
    let member = caller(Some("bob"), Some("team-a"), None);
    let error = assert_write_access(
        &member,
        &team_row,
        &FakeTeams::failing(),
        &access(Vec::new(), false),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(
        error.detail,
        "You do not have permission to modify this memory entry."
    );
    let log = captured.text();
    assert!(log.contains("Error loading team for write-auth check (team_id=team-a)"));
    drop(guard);
}

#[tokio::test]
async fn team_access_check_failure_denies() {
    let team_row = row("mem-1", "note", None, Some("team-a"));
    let teams = FakeTeams::new(Some(team("team-a", Some("org-1"), &[])));
    let member = caller(Some("bob"), Some("team-a"), None);
    let error = assert_write_access(&member, &team_row, &teams, &access(Vec::new(), true))
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(teams.lookups(), vec!["team-a".to_owned()]);
}

#[tokio::test]
async fn team_admin_check_logs_and_denies() {
    let (captured, guard) = capture();
    let team_row = row("mem-1", "note", None, Some("team-a"));
    let teams = FakeTeams::new(Some(team("team-a", Some("org-1"), &[])));
    let member = caller(Some("bob"), Some("team-a"), None);
    let result = assert_write_access(&member, &team_row, &teams, &access(Vec::new(), true)).await;
    assert_eq!(result.unwrap_err().status, StatusCode::FORBIDDEN);
    let log = captured.text();
    assert!(log.contains("Org-admin check skipped during write-auth (team_id=team-a)"));
    drop(guard);
}
