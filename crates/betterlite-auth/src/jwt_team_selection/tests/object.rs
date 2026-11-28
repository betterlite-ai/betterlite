use super::support::rendered;
use crate::jwt_team_selection::object::{ObjectPolicy, validate_object_id};

#[test]
fn object_identity_required() {
    let policy = ObjectPolicy {
        enforce_rbac: true,
        is_proxy_admin: false,
    };
    let (status, error_type, message) =
        rendered(validate_object_id(None, None, policy).expect_err("no identity is refused"));
    assert_eq!(status, 403);
    assert_eq!(error_type, "auth_error");
    assert_eq!(
        message,
        "No user or team id found in token. enforce_rbac is set to True. Token must belong to a proxy admin, team, or user."
    );
}

#[test]
fn object_identity_accepts_user() {
    let policy = ObjectPolicy {
        enforce_rbac: true,
        is_proxy_admin: false,
    };
    assert!(validate_object_id(Some("user-1"), None, policy).is_ok());
}

#[test]
fn object_identity_accepts_team() {
    let policy = ObjectPolicy {
        enforce_rbac: true,
        is_proxy_admin: false,
    };
    assert!(validate_object_id(None, Some("team-a"), policy).is_ok());
}

#[test]
fn object_identity_accepts_admin() {
    let policy = ObjectPolicy {
        enforce_rbac: true,
        is_proxy_admin: true,
    };
    assert!(validate_object_id(None, None, policy).is_ok());
}

#[test]
fn object_identity_not_enforced() {
    let policy = ObjectPolicy {
        enforce_rbac: false,
        is_proxy_admin: false,
    };
    assert!(validate_object_id(None, None, policy).is_ok());
}
