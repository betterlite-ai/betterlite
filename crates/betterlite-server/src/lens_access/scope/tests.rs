#![cfg(test)]

use axum::http::StatusCode;

use crate::lens_access::error::failure_text;
use crate::lens_access::scope::{key_caller, user_scope};
use crate::lens_access::tests::support::auth_key;

const WRITE_DENIED: &str = "Only proxy admins can configure or run Lens";
const ACCESS_DENIED: &str = "Lens requires proxy administrator access";

#[test]
fn proxy_admin_gets_all_teams_for_reads_and_writes() {
    let auth = auth_key(Some("proxy_admin"), Some("team-1"), &[]);
    let read = user_scope(&auth, false).expect("a proxy admin reads");
    assert!(read.all_teams);
    assert_eq!(read.team_id, "");
    assert_eq!(read.api_key_hash, "");
    let write = user_scope(&auth, true).expect("a proxy admin writes");
    assert!(write.all_teams);
}

#[test]
fn viewer_reads_all_teams_and_writes_are_denied() {
    let auth = auth_key(Some("proxy_admin_viewer"), None, &[]);
    let read = user_scope(&auth, false).expect("a viewer reads");
    assert!(read.all_teams);
    let denial = user_scope(&auth, true).expect_err("a viewer cannot write");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), WRITE_DENIED);
}

#[test]
fn org_admin_is_denied() {
    let auth = auth_key(Some("org_admin"), None, &[]);
    let denial = user_scope(&auth, false).expect_err("the role is denied");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), ACCESS_DENIED);
}

#[test]
fn internal_user_is_denied() {
    let auth = auth_key(Some("internal_user"), None, &[]);
    let denial = user_scope(&auth, false).expect_err("the role is denied");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), ACCESS_DENIED);
}

#[test]
fn internal_user_viewer_is_denied() {
    let auth = auth_key(Some("internal_user_viewer"), None, &[]);
    let denial = user_scope(&auth, false).expect_err("the role is denied");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), ACCESS_DENIED);
}

#[test]
fn team_role_is_denied() {
    let auth = auth_key(Some("team"), None, &[]);
    let denial = user_scope(&auth, false).expect_err("the role is denied");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), ACCESS_DENIED);
}

#[test]
fn customer_role_is_denied() {
    let auth = auth_key(Some("customer"), None, &[]);
    let denial = user_scope(&auth, false).expect_err("the role is denied");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), ACCESS_DENIED);
}

#[test]
fn a_key_without_a_role_is_denied() {
    let auth = auth_key(None, None, &[]);
    let denial = user_scope(&auth, false).expect_err("the key is denied");
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(failure_text(&denial), ACCESS_DENIED);
}

#[test]
fn the_key_fields_survive_the_serde_view() {
    let auth = auth_key(Some("team"), Some("team-7"), &["gpt-4o", "claude"]);
    let fields = key_caller(&auth).expect("the key view reads");
    assert_eq!(fields.team_id.as_deref(), Some("team-7"));
    assert_eq!(fields.models, ["gpt-4o", "claude"]);
    assert!(fields.user_role.is_some());
}
