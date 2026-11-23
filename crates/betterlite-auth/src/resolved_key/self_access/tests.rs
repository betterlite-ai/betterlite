#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::allowed_inside_route;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

fn caller(user_role: Option<&str>, user_id: Option<&str>) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(role) = user_role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_owned()));
    }
    if let Some(id) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(id.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .unwrap_or_else(|error| panic!("fixture should construct: {error}"))
}

#[test]
fn proxy_admin_allowed() {
    let admin = caller(Some("proxy_admin"), Some("admin-1"));
    assert!(allowed_inside_route(&admin, Some("user-2")));
    assert!(allowed_inside_route(&admin, None));
}

#[test]
fn admin_viewer_allowed() {
    let viewer = caller(Some("proxy_admin_viewer"), None);
    assert!(allowed_inside_route(&viewer, Some("user-2")));
    assert!(allowed_inside_route(&viewer, None));
}

#[test]
fn same_user_allowed() {
    let user = caller(Some("internal_user"), Some("user-1"));
    assert!(allowed_inside_route(&user, Some("user-1")));
}

#[test]
fn empty_ids_match() {
    let user = caller(Some("internal_user"), Some(""));
    assert!(allowed_inside_route(&user, Some("")));
}

#[test]
fn other_user_refused() {
    let user = caller(Some("internal_user"), Some("user-1"));
    assert!(!allowed_inside_route(&user, Some("user-2")));
}

#[test]
fn missing_requested_id_refused() {
    let user = caller(Some("internal_user"), Some("user-1"));
    assert!(!allowed_inside_route(&user, None));
}

#[test]
fn keyless_user_refused() {
    let keyless = caller(Some("internal_user"), None);
    assert!(!allowed_inside_route(&keyless, None));
    assert!(!allowed_inside_route(&keyless, Some("user-1")));
}

#[rstest]
#[case::org_admin(Some("org_admin"))]
#[case::internal_user(Some("internal_user"))]
#[case::internal_user_viewer(Some("internal_user_viewer"))]
#[case::team(Some("team"))]
#[case::customer(Some("customer"))]
#[case::no_role(None)]
fn other_roles_need_same_user(#[case] role: Option<&str>) {
    let user = caller(role, Some("user-1"));
    assert!(allowed_inside_route(&user, Some("user-1")));
    assert!(!allowed_inside_route(&user, Some("user-2")));
}
