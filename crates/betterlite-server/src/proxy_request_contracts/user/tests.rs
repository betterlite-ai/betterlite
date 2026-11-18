#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_auth::user_roles::UserRole;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use super::{
    ChangePasswordRequest, ChangePasswordResponse, NewUserRequest, NewUserRequestTeam,
    TeamUserRole, UpdateUserFields, UpdateUserRequest,
};
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::tests::{assert_decode, assert_validation, contract};

#[test]
fn new_user_keeps_the_defaults() {
    let request = NewUserRequest::from_value(&contract(r#"{"user_email": "a@example.com"}"#))
        .expect("a new user");
    assert!(request.auto_create_key);
    assert_eq!(request.max_budget, None);
    assert_eq!(request.user_alias, None);
    assert_eq!(request.user_role, None);
    assert_eq!(request.teams, None);
    assert_eq!(request.send_invite_email, None);
    assert_eq!(request.sso_user_id, None);
    assert_eq!(request.organizations, None);
    assert!(request.password.is_none());
    assert_eq!(request.base.spend, None);
    let projection = request.base.project().expect("the resolved base");
    assert_eq!(
        serde_json::to_value(projection.resolved.get("spend")).expect("a value"),
        contract("0.0")
    );
}

#[rstest]
#[case::proxy_admin("proxy_admin", UserRole::ProxyAdmin)]
#[case::proxy_admin_viewer("proxy_admin_viewer", UserRole::ProxyAdminViewer)]
#[case::internal_user("internal_user", UserRole::InternalUser)]
#[case::internal_user_viewer("internal_user_viewer", UserRole::InternalUserViewer)]
fn new_user_accepts_the_four_roles(#[case] text: &str, #[case] role: UserRole) {
    let body = format!(r#"{{"user_email": "a@example.com", "user_role": "{text}"}}"#);
    let request = NewUserRequest::from_value(&contract(&body)).expect("an allowed role");
    assert_eq!(request.user_role, Some(role));
}

#[rstest]
#[case::org_admin("org_admin")]
#[case::team("team")]
#[case::customer("customer")]
fn new_user_rejects_the_other_roles(#[case] role: &str) {
    let body = format!(r#"{{"user_email": "a@example.com", "user_role": "{role}"}}"#);
    let error = NewUserRequest::from_value(&contract(&body)).expect_err("a rejected role");
    assert_validation(&error, &format!("user_role {role} is not allowed"));
}

#[test]
fn new_user_rejects_an_unknown_role() {
    let error = NewUserRequest::from_value(&contract(
        r#"{"user_email": "a@example.com", "user_role": "superuser"}"#,
    ))
    .expect_err("a role that does not exist");
    assert_decode(error);
}

#[rstest]
#[case::empty(r#"{"user_email": "a@example.com", "password": ""}"#)]
#[case::value(r#"{"user_email": "a@example.com", "password": "hunter2"}"#)]
fn new_user_rejects_a_non_null_password(#[case] text: &str) {
    let error = NewUserRequest::from_value(&contract(text)).expect_err("a password");
    assert_validation(
        &error,
        "password cannot be set via /user/new. Users set their own password through an invitation link (POST /invitation/new).",
    );
}

#[test]
fn new_user_accepts_a_null_password() {
    NewUserRequest::from_value(&contract(
        r#"{"user_email": "a@example.com", "password": null}"#,
    ))
    .expect("a null password");
}

#[rstest]
#[case::ids(
    r#"{"user_email": "a@example.com", "teams": ["t1", "t2"]}"#,
    r#"["t1","t2"]"#
)]
#[case::records(
    r#"{"user_email": "a@example.com", "teams": [{"team_id": "t1"}]}"#,
    r#"[{"team_id":"t1","max_budget_in_team":null,"user_role":"user"}]"#
)]
#[case::budget(
    r#"{"user_email": "a@example.com", "teams": [{"team_id": "t1", "max_budget_in_team": 5, "user_role": "admin"}]}"#,
    r#"[{"team_id":"t1","max_budget_in_team":5.0,"user_role":"admin"}]"#
)]
fn new_user_takes_team_ids_or_records(#[case] text: &str, #[case] dumped: &str) {
    let request = NewUserRequest::from_value(&contract(text)).expect("teams");
    assert_eq!(
        serde_json::to_string(&request.teams).expect("the dump"),
        dumped
    );
}

#[test]
fn new_user_rejects_a_mixed_team_list() {
    let error = NewUserRequest::from_value(&contract(
        r#"{"user_email": "a@example.com", "teams": ["t1", {"team_id": "t2"}]}"#,
    ))
    .expect_err("a mixed list");
    assert_decode(error);
}

#[test]
fn new_user_reads_the_flattened_base() {
    let request = NewUserRequest::from_value(&contract(
        r#"{"user_email": "a@example.com", "spend": 3.5, "team_id": "t", "models": ["m"], "max_budget": 10}"#,
    ))
    .expect("a new user");
    assert_eq!(request.base.spend, Some(Some(Amount(Decimal::new(35, 1)))));
    assert_eq!(request.base.team_id, Some(Some("t".to_owned())));
    assert!(matches!(&request.base.models, Some(Some(models)) if models.len() == 1));
    assert_eq!(request.max_budget, Some(Amount(Decimal::new(10, 0))));
    assert_eq!(request.base.max_budget, None);
}

#[test]
fn new_user_reads_a_blank_max_budget_as_null() {
    let request = NewUserRequest::from_value(&contract(
        r#"{"user_email": "a@example.com", "max_budget": ""}"#,
    ))
    .expect("a blank max_budget");
    assert_eq!(request.max_budget, None);
}

#[test]
fn new_user_ignores_unknown_keys() {
    NewUserRequest::from_value(&contract(
        r#"{"user_email": "a@example.com", "bogus": {"a": 1}}"#,
    ))
    .expect("unknown keys are ignored");
}

#[test]
fn team_record_defaults_to_the_user_role() {
    let team = NewUserRequestTeam::from_value(&contract(r#"{"team_id": "t"}"#)).expect("a team");
    assert_eq!(team.team_id, "t");
    assert_eq!(team.user_role, TeamUserRole::User);
    assert_eq!(team.max_budget_in_team, None);
}

#[rstest]
#[case::empty("{}")]
#[case::both_null(r#"{"user_id": null, "user_email": null}"#)]
#[case::id_null(r#"{"user_id": null}"#)]
fn update_user_rejects_a_body_without_a_selector(#[case] text: &str) {
    let error = UpdateUserRequest::from_value(&contract(text)).expect_err("no selector");
    assert_validation(&error, "Either user id or user email must be provided");
}

#[rstest]
#[case::user_id(r#"{"user_id": "u"}"#)]
#[case::user_email(r#"{"user_email": "a@example.com"}"#)]
#[case::empty_id(r#"{"user_id": ""}"#)]
#[case::empty_email(r#"{"user_email": ""}"#)]
fn update_user_accepts_a_selector(#[case] text: &str) {
    UpdateUserRequest::from_value(&contract(text)).expect("a selector");
}

#[test]
fn update_user_rejects_a_role_outside_the_four() {
    let error =
        UpdateUserRequest::from_value(&contract(r#"{"user_id": "u", "user_role": "org_admin"}"#))
            .expect_err("a rejected role");
    assert_validation(&error, "user_role org_admin is not allowed");
}

#[test]
fn update_fields_reject_a_role_outside_the_four() {
    let error = UpdateUserFields::from_value(&contract(r#"{"user_role": "team"}"#))
        .expect_err("a rejected role");
    assert_validation(&error, "user_role team is not allowed");
}

#[test]
fn update_user_hides_the_password_in_debug() {
    let request =
        UpdateUserRequest::from_value(&contract(r#"{"user_id": "u", "password": "hunter2"}"#))
            .expect("a password change");
    assert!(request.fields.password.is_some());
    assert!(!format!("{request:?}").contains("hunter2"));
}

#[test]
fn update_user_reads_the_user_fields() {
    let request = UpdateUserRequest::from_value(&contract(
        r#"{
            "user_email": "a@example.com",
            "user_alias": "a",
            "user_role": "internal_user",
            "metadata": {"a": 1}
        }"#,
    ))
    .expect("an update");
    assert_eq!(request.user_email.as_deref(), Some("a@example.com"));
    assert_eq!(request.user_id, None);
    assert_eq!(request.fields.user_alias.as_deref(), Some("a"));
    assert_eq!(request.fields.user_role, Some(UserRole::InternalUser));
    assert!(request.fields.metadata.is_some());
    assert_eq!(request.fields.base.metadata, None);
}

#[test]
fn update_user_records_which_fields_were_sent() {
    let request = UpdateUserRequest::from_value(&contract(
        r#"{"user_id": "u", "spend": null, "max_budget": 5}"#,
    ))
    .expect("an update");
    assert_eq!(request.fields.spend, None);
    assert_eq!(request.fields.max_budget, Some(Amount(Decimal::new(5, 0))));
    assert_eq!(
        request.fields.supplied,
        BTreeSet::from(["max_budget".to_owned(), "spend".to_owned()])
    );
    assert_eq!(request.fields.base.spend, None);
}

#[test]
fn update_user_reads_a_blank_max_budget_as_null() {
    let request = UpdateUserRequest::from_value(&contract(r#"{"user_id": "u", "max_budget": ""}"#))
        .expect("a blank max_budget");
    assert_eq!(request.fields.max_budget, None);
    assert_eq!(
        request.fields.supplied,
        BTreeSet::from(["max_budget".to_owned()])
    );
}

#[test]
fn change_password_hides_both_passwords() {
    let request = ChangePasswordRequest::from_value(&contract(
        r#"{"current_password": "old-secret", "new_password": "new-secret"}"#,
    ))
    .expect("a password change");
    assert_eq!(request.current_password.expose_secret(), "old-secret");
    assert_eq!(request.new_password.expose_secret(), "new-secret");
    let text = format!("{request:?}");
    assert!(!text.contains("old-secret"));
    assert!(!text.contains("new-secret"));
}

#[rstest]
#[case::empty("{}")]
#[case::one(r#"{"current_password": "old"}"#)]
#[case::null(r#"{"current_password": null, "new_password": "new"}"#)]
fn change_password_requires_both_passwords(#[case] text: &str) {
    let error = ChangePasswordRequest::from_value(&contract(text)).expect_err("a password");
    assert_decode(error);
}

#[test]
fn change_password_response_carries_the_user_and_message() {
    let response =
        ChangePasswordResponse::from_value(&contract(r#"{"user_id": "u", "message": "done"}"#))
            .expect("a response");
    assert_eq!(response.user_id, "u");
    assert_eq!(response.message, "done");
}
