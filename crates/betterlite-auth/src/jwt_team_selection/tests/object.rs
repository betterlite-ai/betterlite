use super::support::rendered;
use crate::jwt_team_selection::object::validate_object_id;

#[test]
fn object_identity_required() {
    let (status, error_type, message) =
        rendered(validate_object_id(None, None, true, false).expect_err("no identity is refused"));
    assert_eq!(status, 403);
    assert_eq!(error_type, "auth_error");
    assert_eq!(
        message,
        "No user or team id found in token. enforce_rbac is set to True. Token must belong to a proxy admin, team, or user."
    );
}

#[test]
fn object_identity_accepts_user() {
    assert!(validate_object_id(Some("user-1"), None, true, false).is_ok());
}

#[test]
fn object_identity_accepts_team() {
    assert!(validate_object_id(None, Some("team-a"), true, false).is_ok());
}

#[test]
fn object_identity_accepts_admin() {
    assert!(validate_object_id(None, None, true, true).is_ok());
}

#[test]
fn object_identity_not_enforced() {
    assert!(validate_object_id(None, None, false, false).is_ok());
}
