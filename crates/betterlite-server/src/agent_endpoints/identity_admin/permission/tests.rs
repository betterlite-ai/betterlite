#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::check_agent_management_permission;
use crate::agent_endpoints::identity_admin::error::Detail;

fn caller(role: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(
        String::from("api_key"),
        JsonData::String(String::from("sk-test")),
    );
    if let Some(given) = role {
        fields.insert(
            String::from("user_role"),
            JsonData::String(given.to_owned()),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

#[test]
fn a_proxy_admin_may_manage_agents() {
    check_agent_management_permission(&caller(Some("proxy_admin"))).unwrap();
}

#[rstest]
#[case::viewer(Some("proxy_admin_viewer"))]
#[case::org_admin(Some("org_admin"))]
#[case::internal_user(Some("internal_user"))]
#[case::internal_viewer(Some("internal_user_viewer"))]
#[case::team(Some("team"))]
#[case::customer(Some("customer"))]
#[case::no_role(None)]
fn every_other_role_is_forbidden(#[case] role: Option<&str>) {
    let error = check_agent_management_permission(&caller(role)).unwrap_err();
    let shown = role.unwrap_or("None");
    assert_eq!(error.status, 403);
    assert_eq!(
        error.detail,
        Detail::Object {
            error: format!(
                "Only proxy admins can create, update, or delete agents. Your role={shown}"
            ),
        }
    );
}
