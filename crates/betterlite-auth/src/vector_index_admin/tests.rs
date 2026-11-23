#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::user_roles::UserRole;
use crate::vector_index_admin::{
    IndexAdminError, IndexOperation, auth_is_proxy_admin, require_index_admin,
};

fn construct(role: Option<UserRole>) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(assigned) = role {
        fields.insert(
            String::from("user_role"),
            JsonData::String(assigned.to_string()),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .unwrap_or_else(|error| panic!("fixture should construct: {error}"))
}

fn refusal_message(operation: &str) -> String {
    format!(
        "Only proxy admins can {operation} vector store indexes. Contact your LiteLLM administrator."
    )
}

#[rstest]
#[case::proxy_admin(Some(UserRole::ProxyAdmin), true)]
#[case::proxy_admin_viewer(Some(UserRole::ProxyAdminViewer), false)]
#[case::org_admin(Some(UserRole::OrgAdmin), false)]
#[case::internal_user(Some(UserRole::InternalUser), false)]
#[case::internal_user_viewer(Some(UserRole::InternalUserViewer), false)]
#[case::team(Some(UserRole::Team), false)]
#[case::customer(Some(UserRole::Customer), false)]
#[case::no_role(None, false)]
fn only_a_proxy_admin_administers_indexes(#[case] role: Option<UserRole>, #[case] expected: bool) {
    assert_eq!(auth_is_proxy_admin(&construct(role)), expected);
}

#[rstest]
#[case::create(IndexOperation::Create, "create")]
#[case::delete(IndexOperation::Delete, "delete")]
#[case::update(IndexOperation::Update, "update")]
#[case::list(IndexOperation::List, "list")]
fn every_operation_reads_as_litellm_spells_it(
    #[case] operation: IndexOperation,
    #[case] name: &str,
) {
    assert_eq!(operation.to_string(), name);
}

#[rstest]
#[case::create(IndexOperation::Create)]
#[case::delete(IndexOperation::Delete)]
#[case::update(IndexOperation::Update)]
#[case::list(IndexOperation::List)]
fn a_proxy_admin_passes_every_gate(#[case] operation: IndexOperation) {
    require_index_admin(&construct(Some(UserRole::ProxyAdmin)), operation).unwrap();
}

#[rstest]
#[case::create(IndexOperation::Create, "create")]
#[case::delete(IndexOperation::Delete, "delete")]
#[case::update(IndexOperation::Update, "update")]
#[case::list(IndexOperation::List, "list")]
fn a_refusal_names_the_operation(#[case] operation: IndexOperation, #[case] name: &str) {
    let refused = construct(Some(UserRole::ProxyAdminViewer));
    let error = require_index_admin(&refused, operation).unwrap_err();
    assert_eq!(error.to_string(), refusal_message(name));
    assert!(matches!(error, IndexAdminError::NotProxyAdmin(named) if named == operation));
}

#[rstest]
#[case::proxy_admin_viewer(UserRole::ProxyAdminViewer)]
#[case::org_admin(UserRole::OrgAdmin)]
#[case::internal_user(UserRole::InternalUser)]
#[case::internal_user_viewer(UserRole::InternalUserViewer)]
#[case::team(UserRole::Team)]
#[case::customer(UserRole::Customer)]
fn every_other_role_is_refused(#[case] role: UserRole) {
    let refused = construct(Some(role));
    let error = require_index_admin(&refused, IndexOperation::Create).unwrap_err();
    assert_eq!(error.to_string(), refusal_message("create"));
}

#[test]
fn a_caller_without_a_role_is_refused() {
    let refused = construct(None);
    let error = require_index_admin(&refused, IndexOperation::List).unwrap_err();
    assert_eq!(error.to_string(), refusal_message("list"));
}
