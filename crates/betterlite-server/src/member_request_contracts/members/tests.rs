#![cfg(test)]

use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{
    MemberAddRequest, OneOrMany, OrgMember, OrgMemberAddRequest, OrganizationMemberAddRequest,
    OrganizationMemberDeleteRequest, OrganizationMemberUpdateRequest, ROLES_WITHIN_ORG,
    TeamMemberAddRequest, TeamMemberDeleteRequest, TeamMemberUpdateRequest,
};
use crate::member_request_contracts::MemberContractError;

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn org_member(body: Value) -> Result<OrgMember, MemberContractError> {
    OrgMember::from_value(&body)
}

fn member_add(body: Value) -> Result<MemberAddRequest, MemberContractError> {
    MemberAddRequest::from_value(&body)
}

fn member_update(body: Value) -> Result<TeamMemberUpdateRequest, MemberContractError> {
    TeamMemberUpdateRequest::from_value(&body)
}

fn org_member_update(body: Value) -> Result<OrganizationMemberUpdateRequest, MemberContractError> {
    OrganizationMemberUpdateRequest::from_value(&body)
}

fn message(error: MemberContractError) -> String {
    match error {
        MemberContractError::Validation { message } => message,
        MemberContractError::Decode { source } => {
            panic!("expected a validation error, got {source}")
        }
    }
}

fn team_member(role: MemberRole) -> Member {
    Member {
        base: MemberBase {
            user_id: Some("user-1".to_owned()),
            user_email: None,
        },
        role,
    }
}

#[test]
fn exports_the_org_roles_in_order() {
    assert_eq!(
        ROLES_WITHIN_ORG,
        [
            UserRole::OrgAdmin,
            UserRole::InternalUser,
            UserRole::InternalUserViewer,
        ]
    );
}

#[rstest]
#[case("org_admin", UserRole::OrgAdmin)]
#[case("internal_user", UserRole::InternalUser)]
#[case("internal_user_viewer", UserRole::InternalUserViewer)]
fn accepts_the_three_org_roles(#[case] text: &str, #[case] expected: UserRole) {
    let member = org_member(json!({ "user_email": "a@b.c", "role": text })).unwrap();
    assert_eq!(member.role, expected);
    assert_eq!(member.base.user_email, Some("a@b.c".to_owned()));
    assert_eq!(member.base.user_id, None);
}

#[test]
fn rejects_a_team_role_for_an_org_member() {
    let error = org_member(json!({ "user_id": "user-1", "role": "team" })).unwrap_err();
    let expected = "Input should be 'org_admin', 'internal_user' or 'internal_user_viewer'";
    assert_eq!(message(error), expected);
}

#[test]
fn rejects_an_unknown_org_member_role() {
    let error = org_member(json!({ "user_id": "user-1", "role": "owner" })).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_an_org_member_without_an_identity() {
    let error = org_member(json!({ "role": "org_admin" })).unwrap_err();
    assert_eq!(
        message(error),
        "Either user id or user email must be provided"
    );
}

#[test]
fn rejects_an_org_member_that_is_not_an_object() {
    let error = org_member(json!("user-1")).unwrap_err();
    assert_eq!(message(error), "input needs to be a dictionary");
}

#[test]
fn takes_a_single_member() {
    let request =
        member_add(json!({ "member": { "user_id": "user-1", "role": "admin" } })).unwrap();
    assert_eq!(
        request.member,
        OneOrMany::One(team_member(MemberRole::Admin))
    );
}

#[test]
fn takes_a_member_list_in_order() {
    let request = member_add(json!({
        "member": [
            { "user_id": "user-1", "role": "admin" },
            { "user_email": "a@b.c", "role": "user" },
        ],
    }))
    .unwrap();
    let second = Member {
        base: MemberBase {
            user_id: None,
            user_email: Some("a@b.c".to_owned()),
        },
        role: MemberRole::User,
    };
    assert_eq!(
        request.member,
        OneOrMany::Many(vec![team_member(MemberRole::Admin), second])
    );
}

#[test]
fn takes_an_empty_member_list() {
    let request = member_add(json!({ "member": [] })).unwrap();
    assert_eq!(request.member, OneOrMany::Many(Vec::new()));
}

#[test]
fn rejects_a_mixed_member_list() {
    let error = member_add(json!({
        "member": [{ "user_id": "user-1", "role": "admin" }, "user-2"],
    }))
    .unwrap_err();
    assert_eq!(message(error), "input needs to be a dictionary");
}

#[test]
fn reports_a_member_without_a_role_as_a_validation_error() {
    let error = member_add(json!({ "member": { "user_id": "user-1" } })).unwrap_err();
    let text = message(error);
    assert!(
        text.starts_with("stored row is invalid JSON: missing field `role`"),
        "{text}"
    );
}

#[test]
fn reports_a_member_with_an_unknown_role_as_a_validation_error() {
    let error =
        member_add(json!({ "member": { "user_id": "user-1", "role": "owner" } })).unwrap_err();
    let text = message(error);
    assert!(
        text.contains("stored row is invalid JSON: unknown variant `owner`"),
        "{text}"
    );
}

#[test]
fn rejects_a_member_without_an_identity() {
    let error = member_add(json!({ "member": { "role": "admin" } })).unwrap_err();
    assert_eq!(
        message(error),
        "Either user id or user email must be provided"
    );
}

#[test]
fn rejects_a_body_without_member() {
    let error = member_add(json!({})).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn serializes_a_single_member_as_an_object() {
    let request =
        member_add(json!({ "member": { "user_id": "user-1", "role": "admin" } })).unwrap();
    let dumped = serde_json::to_value(&request).unwrap();
    assert_eq!(
        dumped,
        json!({ "member": { "user_id": "user-1", "user_email": null, "role": "admin" } })
    );
}

#[test]
fn serializes_a_member_list_as_an_array() {
    let request = member_add(json!({
        "member": [{ "user_id": "user-1", "role": "user" }],
    }))
    .unwrap();
    let dumped = serde_json::to_value(&request).unwrap();
    assert_eq!(
        dumped,
        json!({ "member": [{ "user_id": "user-1", "user_email": null, "role": "user" }] })
    );
}

#[test]
fn decodes_the_team_member_add_fields() {
    let request = TeamMemberAddRequest::from_value(&json!({
        "member": { "user_id": "user-1", "role": "user" },
        "team_id": "team-1",
        "max_budget_in_team": 12.5,
        "budget_duration": "1d",
        "allowed_models": ["gpt-4"],
    }))
    .unwrap();
    assert_eq!(
        request.member,
        OneOrMany::One(team_member(MemberRole::User))
    );
    assert_eq!(request.team_id, "team-1");
    assert_eq!(request.max_budget_in_team, Some(Decimal::new(125, 1)));
    assert_eq!(request.budget_duration, Some("1d".to_owned()));
    assert_eq!(request.allowed_models, Some(vec!["gpt-4".to_owned()]));
}

#[test]
fn defaults_the_optional_team_member_add_fields() {
    let request = TeamMemberAddRequest::from_value(&json!({
        "member": { "user_id": "user-1", "role": "user" },
        "team_id": "team-1",
    }))
    .unwrap();
    assert_eq!(request.max_budget_in_team, None);
    assert_eq!(request.budget_duration, None);
    assert_eq!(request.allowed_models, None);
}

#[test]
fn rejects_a_team_member_add_without_a_team_id() {
    let error = TeamMemberAddRequest::from_value(&json!({
        "member": { "user_id": "user-1", "role": "user" },
    }))
    .unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn takes_org_members_one_or_many() {
    let request = OrgMemberAddRequest::from_value(&json!({
        "member": [
            { "user_id": "user-1", "role": "org_admin" },
            { "user_email": "a@b.c", "role": "internal_user" },
        ],
    }))
    .unwrap();
    let OneOrMany::Many(members) = request.member else {
        panic!("expected a member list");
    };
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].role, UserRole::OrgAdmin);
    assert_eq!(members[1].role, UserRole::InternalUser);
}

#[test]
fn rejects_a_mixed_org_member_list() {
    let error = OrgMemberAddRequest::from_value(&json!({
        "member": [{ "user_id": "user-1", "role": "org_admin" }, 1],
    }))
    .unwrap_err();
    assert_eq!(message(error), "input needs to be a dictionary");
}

#[test]
fn decodes_the_organization_member_add_fields() {
    let request = OrganizationMemberAddRequest::from_value(&json!({
        "member": { "user_id": "user-1", "role": "org_admin" },
        "organization_id": "org-1",
        "max_budget_in_organization": 12.5,
    }))
    .unwrap();
    assert_eq!(request.organization_id, "org-1");
    assert_eq!(
        request.max_budget_in_organization,
        Some(Decimal::new(125, 1))
    );
    assert!(matches!(request.member, OneOrMany::One(_)));
}

#[test]
fn rejects_an_organization_member_add_without_an_organization() {
    let error = OrganizationMemberAddRequest::from_value(&json!({
        "member": { "user_id": "user-1", "role": "org_admin" },
    }))
    .unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[rstest]
#[case("user_id", "user-1")]
#[case("user_email", "a@b.c")]
fn decodes_a_team_member_delete_identity(#[case] field: &str, #[case] value: &str) {
    let mut body = json!({ "team_id": "team-1" });
    body[field] = json!(value);
    let request = TeamMemberDeleteRequest::from_value(&body).unwrap();
    assert_eq!(request.team_id, "team-1");
}

#[test]
fn rejects_a_team_member_delete_without_an_identity() {
    let error = TeamMemberDeleteRequest::from_value(&json!({ "team_id": "team-1" })).unwrap_err();
    assert_eq!(
        message(error),
        "Either user id or user email must be provided"
    );
}

#[test]
fn rejects_a_team_member_delete_without_a_team_id() {
    let error = TeamMemberDeleteRequest::from_value(&json!({ "user_id": "user-1" })).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[rstest]
#[case("user_id", "user-1")]
#[case("user_email", "a@b.c")]
fn decodes_an_organization_member_delete_identity(#[case] field: &str, #[case] value: &str) {
    let mut body = json!({ "organization_id": "org-1" });
    body[field] = json!(value);
    let request = OrganizationMemberDeleteRequest::from_value(&body).unwrap();
    assert_eq!(request.organization_id, "org-1");
}

#[test]
fn rejects_an_organization_member_delete_without_an_identity() {
    let error = OrganizationMemberDeleteRequest::from_value(&json!({ "organization_id": "org-1" }))
        .unwrap_err();
    assert_eq!(
        message(error),
        "Either user id or user email must be provided"
    );
}

#[test]
fn rejects_an_organization_member_delete_without_an_organization() {
    let error =
        OrganizationMemberDeleteRequest::from_value(&json!({ "user_id": "user-1" })).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[rstest]
#[case("null")]
#[case("[]")]
fn rejects_a_member_request_body_that_is_not_an_object(#[case] body: &str) {
    let value = parse(body);
    let errors = [
        TeamMemberDeleteRequest::from_value(&value).unwrap_err(),
        TeamMemberUpdateRequest::from_value(&value).unwrap_err(),
        OrganizationMemberDeleteRequest::from_value(&value).unwrap_err(),
        OrganizationMemberUpdateRequest::from_value(&value).unwrap_err(),
    ];
    for error in errors {
        assert!(matches!(error, MemberContractError::Validation { .. }));
    }
}

#[test]
fn keeps_a_team_member_update_with_both_temp_budget_fields() {
    let request = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "temp_budget_increase": 12.5,
        "temp_budget_expiry": "2026-03-01T00:00:00",
    }))
    .unwrap();
    assert_eq!(request.temp_budget_increase, Some(Decimal::new(125, 1)));
    assert!(request.temp_budget_expiry.is_some());
}

#[test]
fn keeps_a_team_member_update_with_neither_temp_budget_field() {
    let request = member_update(json!({ "user_id": "user-1", "team_id": "team-1" })).unwrap();
    assert_eq!(request.temp_budget_increase, None);
    assert_eq!(request.temp_budget_expiry, None);
}

#[test]
fn rejects_a_temp_budget_increase_without_an_expiry() {
    let error = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "temp_budget_increase": 12.5,
    }))
    .unwrap_err();
    assert_eq!(
        message(error),
        "temp_budget_increase and temp_budget_expiry must be set together"
    );
}

#[test]
fn rejects_a_temp_budget_expiry_without_an_increase() {
    let error = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "temp_budget_expiry": "2026-03-01T00:00:00",
    }))
    .unwrap_err();
    assert_eq!(
        message(error),
        "temp_budget_increase and temp_budget_expiry must be set together"
    );
}

#[test]
fn rejects_a_negative_temp_budget_increase_before_pairing() {
    let error = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "temp_budget_increase": -1,
    }))
    .unwrap_err();
    assert_eq!(message(error), "Input should be greater than or equal to 0");
}

#[test]
fn keeps_a_zero_temp_budget_increase() {
    let request = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "temp_budget_increase": 0,
        "temp_budget_expiry": "2026-03-01T00:00:00",
    }))
    .unwrap();
    assert_eq!(request.temp_budget_increase, Some(Decimal::ZERO));
}

#[rstest]
#[case("admin", MemberRole::Admin)]
#[case("user", MemberRole::User)]
fn decodes_the_team_member_role(#[case] text: &str, #[case] expected: MemberRole) {
    let request = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "role": text,
    }))
    .unwrap();
    assert_eq!(request.role, Some(expected));
}

#[test]
fn rejects_an_unknown_team_member_role() {
    let error = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "role": "owner",
    }))
    .unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn decodes_the_team_member_update_limits() {
    let request = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "max_budget_in_team": 12.5,
        "tpm_limit": 100,
        "rpm_limit": 10,
        "budget_duration": "1d",
        "allowed_models": [],
    }))
    .unwrap();
    assert_eq!(request.max_budget_in_team, Some(Decimal::new(125, 1)));
    assert_eq!(request.tpm_limit, Some(100));
    assert_eq!(request.rpm_limit, Some(10));
    assert_eq!(request.budget_duration, Some("1d".to_owned()));
    assert_eq!(request.allowed_models, Some(Vec::new()));
}

#[test]
fn decodes_the_organization_member_update_fields() {
    let request = org_member_update(json!({
        "user_id": "user-1",
        "organization_id": "org-1",
        "max_budget_in_organization": 12.5,
        "role": "internal_user_viewer",
    }))
    .unwrap();
    assert_eq!(request.base.organization_id, "org-1");
    assert_eq!(
        request.max_budget_in_organization,
        Some(Decimal::new(125, 1))
    );
    assert_eq!(request.role, Some(UserRole::InternalUserViewer));
}

#[test]
fn keeps_a_null_organization_member_role() {
    let request = org_member_update(json!({
        "user_id": "user-1",
        "organization_id": "org-1",
        "role": null,
    }))
    .unwrap();
    assert_eq!(request.role, None);
}

#[test]
fn rejects_a_role_outside_the_org() {
    let error = org_member_update(json!({
        "user_id": "user-1",
        "organization_id": "org-1",
        "role": "team",
    }))
    .unwrap_err();
    let expected =
        "Invalid role. Must be one of: ['org_admin', 'internal_user', 'internal_user_viewer']";
    assert_eq!(message(error), expected);
}

#[test]
fn checks_the_identity_before_the_org_role() {
    let error =
        org_member_update(json!({ "organization_id": "org-1", "role": "team" })).unwrap_err();
    assert_eq!(
        message(error),
        "Either user id or user email must be provided"
    );
}

#[test]
fn parses_a_temp_budget_expiry_timestamp() {
    let request = member_update(json!({
        "user_id": "user-1",
        "team_id": "team-1",
        "temp_budget_increase": 1,
        "temp_budget_expiry": parse("\"2026-03-01T00:00:00Z\""),
    }))
    .unwrap();
    assert!(request.temp_budget_expiry.is_some());
}
