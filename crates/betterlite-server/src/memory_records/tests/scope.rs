use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;

use super::support::caller;
use crate::memory_records::scope::{Scope, resolve_scope};

#[test]
fn admin_scope_override() {
    let admin = caller(Some("alice"), Some("team-a"), Some(UserRole::ProxyAdmin));
    assert_eq!(
        resolve_scope(&admin, Some("bob"), Some("team-b")).unwrap(),
        Scope {
            user_id: Some("bob".to_owned()),
            team_id: Some("team-b".to_owned()),
        }
    );
    assert_eq!(
        resolve_scope(&admin, None, None).unwrap(),
        Scope {
            user_id: Some("alice".to_owned()),
            team_id: Some("team-a".to_owned()),
        }
    );
}

#[test]
fn non_admin_scope_mismatch_403() {
    let customer = caller(Some("alice"), Some("team-a"), Some(UserRole::Customer));

    let user_error = resolve_scope(&customer, Some("bob"), None).unwrap_err();
    assert_eq!(user_error.status, StatusCode::FORBIDDEN);
    assert_eq!(
        user_error.detail,
        "Only proxy admins may set user_id to a different user."
    );

    let team_error = resolve_scope(&customer, Some("alice"), Some("team-b")).unwrap_err();
    assert_eq!(team_error.status, StatusCode::FORBIDDEN);
    assert_eq!(
        team_error.detail,
        "Only proxy admins may set team_id to a different team."
    );

    assert_eq!(
        resolve_scope(&customer, Some("alice"), Some("team-a")).unwrap(),
        Scope {
            user_id: Some("alice".to_owned()),
            team_id: Some("team-a".to_owned()),
        }
    );
}

#[test]
fn non_admin_orphan_creation_400() {
    let anonymous = caller(None, None, None);
    let error = resolve_scope(&anonymous, None, None).unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error.detail,
        "Cannot create a memory entry without a user_id or team_id. Authenticate with a key that has a user_id or team_id, or call as a proxy admin."
    );

    let teammate = caller(None, Some("team-a"), Some(UserRole::Customer));
    assert_eq!(
        resolve_scope(&teammate, None, None).unwrap(),
        Scope {
            user_id: None,
            team_id: Some("team-a".to_owned()),
        }
    );
}

#[test]
fn admin_orphan_allowed() {
    let admin = caller(None, None, Some(UserRole::ProxyAdmin));
    assert_eq!(
        resolve_scope(&admin, None, None).unwrap(),
        Scope {
            user_id: None,
            team_id: None,
        }
    );
}

#[test]
fn null_does_not_erase_caller_identity() {
    let admin = caller(Some("alice"), Some("team-a"), Some(UserRole::ProxyAdmin));
    let admin_scope = resolve_scope(&admin, None, None).unwrap();
    assert_eq!(admin_scope.user_id.as_deref(), Some("alice"));
    assert_eq!(admin_scope.team_id.as_deref(), Some("team-a"));

    let customer = caller(Some("alice"), Some("team-a"), Some(UserRole::Customer));
    let customer_scope = resolve_scope(&customer, None, None).unwrap();
    assert_eq!(customer_scope.user_id.as_deref(), Some("alice"));
    assert_eq!(customer_scope.team_id.as_deref(), Some("team-a"));
}
