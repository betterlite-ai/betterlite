#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    CredentialRef, EndUserIdentity, NetworkContext, OrganizationIdentity, Principal,
    ProjectIdentity, TeamIdentity, UserIdentity,
};
use crate::auth_identity::roles::{Role, TeamRole};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

fn fresh_principal() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"sub","auth_method":"api_key"}"#)
        .unwrap()
}

#[test]
fn identity_required_ids() {
    serde_json::from_str::<UserIdentity>("{}").unwrap_err();
    serde_json::from_str::<OrganizationIdentity>("{}").unwrap_err();
    serde_json::from_str::<ProjectIdentity>("{}").unwrap_err();
    serde_json::from_str::<TeamIdentity>("{}").unwrap_err();
    serde_json::from_str::<EndUserIdentity>("{}").unwrap_err();
}

#[test]
fn user_identity_nullable_fields() {
    let identity: UserIdentity = serde_json::from_str(r#"{"id":"u1"}"#).unwrap();
    assert_eq!(identity.external_id, None);
    assert_eq!(identity.user_name, None);
    assert_eq!(identity.email, None);
    assert_eq!(identity.display_name, None);
}

#[test]
fn organization_and_project_name_defaults() {
    let organization: OrganizationIdentity = serde_json::from_str(r#"{"id":"o1"}"#).unwrap();
    let project: ProjectIdentity = serde_json::from_str(r#"{"id":"p1"}"#).unwrap();
    assert_eq!(organization.name, None);
    assert_eq!(project.name, None);
}

#[test]
fn team_name_and_member_defaults() {
    let team: TeamIdentity = serde_json::from_str(r#"{"id":"t1"}"#).unwrap();
    assert_eq!(team.name, None);
    assert_eq!(team.role, TeamRole::Member);
}

#[test]
fn credential_ref_null_ids() {
    let credential = CredentialRef::default();
    assert_eq!(credential.key_id, None);
    assert_eq!(credential.token_id, None);
}

#[test]
fn network_default_fields() {
    let network = NetworkContext::default();
    assert_eq!(network.client_ip, None);
    assert_eq!(network.host, None);
    assert!(!network.via_trusted_proxy);
}

#[test]
fn network_strings_not_normalized() {
    let network: NetworkContext =
        serde_json::from_str(r#"{"client_ip":" 10.0.0.1 ","host":"Example.COM"}"#).unwrap();
    assert_eq!(network.client_ip, Some(" 10.0.0.1 ".to_owned()));
    assert_eq!(network.host, Some("Example.COM".to_owned()));
}

#[test]
fn identity_empty_strings_and_unknown_extras() {
    let identity: UserIdentity =
        serde_json::from_str(r#"{"id":"","email":"","unexpected_field":"ignored"}"#).unwrap();
    assert_eq!(identity.id, "");
    assert_eq!(identity.email, Some(String::new()));
}

#[test]
fn principal_three_required_fields() {
    serde_json::from_str::<Principal>("{}").unwrap_err();
    serde_json::from_str::<Principal>(r#"{"principal_type":"human"}"#).unwrap_err();
    serde_json::from_str::<Principal>(r#"{"principal_type":"human","subject":"sub"}"#).unwrap_err();
    serde_json::from_str::<Principal>(
        r#"{"principal_type":"human","subject":"sub","auth_method":"api_key"}"#,
    )
    .unwrap();
}

#[test]
fn principal_nullable_identity_defaults() {
    let principal = fresh_principal();
    assert_eq!(principal.issuer, None);
    assert_eq!(principal.user, None);
    assert_eq!(principal.organization, None);
    assert_eq!(principal.project, None);
    assert_eq!(principal.end_user, None);
}

#[test]
fn principal_fresh_empty_sequences() {
    let principal = fresh_principal();
    assert!(principal.audience.is_empty());
    assert!(principal.teams.is_empty());
    assert!(principal.roles.is_empty());
    assert!(principal.scopes.is_empty());
}

#[test]
fn principal_fresh_credential_and_network() {
    let principal = fresh_principal();
    assert_eq!(principal.credential_ref, CredentialRef::default());
    assert_eq!(principal.network, NetworkContext::default());
}

#[test]
fn principal_sequence_order_and_duplicates() {
    let mut principal = fresh_principal();
    principal.roles = vec![Role::OrgAdmin, Role::OrgAdmin, Role::TeamMember];
    assert_eq!(
        principal.roles,
        vec![Role::OrgAdmin, Role::OrgAdmin, Role::TeamMember]
    );
}

#[test]
fn principal_top_level_freeze_is_shallow() {
    let mut first = fresh_principal();
    let second = fresh_principal();
    first.roles.push(Role::OrgAdmin);
    first.network.client_ip = Some("10.0.0.1".to_owned());
    assert!(second.roles.is_empty());
    assert_eq!(second.network.client_ip, None);
}

#[test]
fn source_key_excluded_from_json_and_repr() {
    let mut principal = fresh_principal();
    let fields = BTreeMap::from([(
        "api_key".to_owned(),
        JsonData::String("sk-secret-value".to_owned()),
    )]);
    principal.source_key.0 = Some(UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap());
    let encoded = serde_json::to_string(&principal).unwrap();
    assert!(!encoded.contains("source_key"));
    let repr = format!("{principal:?}");
    assert!(!repr.contains("sk-secret-value"));
    assert!(repr.contains("SourceKey(..)"));
}
