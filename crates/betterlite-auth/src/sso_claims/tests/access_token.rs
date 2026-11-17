use rstest::rstest;

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::handler::JwtHandler;
use crate::sso_claims::access_token::process_sso_jwt_access_token;
use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

use super::support::{document, identity, mappings, segment, strings, team_handler, token};

fn processed(
    access_token: Option<&str>,
    sso_jwt_handler: Option<&JwtHandler>,
    identity: Option<OpenIdIdentity>,
    role_mappings: Option<&RoleMappings>,
) -> (Option<OpenIdIdentity>, Option<TypedValue>) {
    process_sso_jwt_access_token(
        access_token,
        sso_jwt_handler,
        identity,
        role_mappings,
        "role",
    )
    .unwrap()
}

#[test]
fn an_opaque_token_adds_nothing() {
    let (updated, payload) = processed(
        Some("opaque-token"),
        Some(&team_handler("team_ids")),
        Some(identity()),
        None,
    );
    assert!(payload.is_none());
    let filled = updated.unwrap();
    assert!(filled.team_ids.is_empty());
    assert!(filled.user_role.is_none());
}

#[test]
fn a_token_whose_header_pyjwt_refuses_adds_nothing() {
    let access_token = format!(
        "{}.{}.{}",
        segment(r#"{"alg":"HS256","kid":7}"#),
        segment("{}"),
        segment("signature")
    );
    let (updated, payload) = processed(
        Some(&access_token),
        Some(&team_handler("team_ids")),
        Some(identity()),
        None,
    );
    assert!(payload.is_none());
    assert!(updated.unwrap().team_ids.is_empty());
}

#[rstest]
#[case(None)]
#[case(Some(""))]
fn a_missing_or_empty_token_adds_nothing(#[case] access_token: Option<&str>) {
    let (updated, payload) = processed(
        access_token,
        Some(&team_handler("team_ids")),
        Some(identity()),
        None,
    );
    assert!(payload.is_none());
    assert!(updated.is_some());
}

#[test]
fn a_missing_identity_has_nothing_to_update() {
    let access_token = token(r#"{"team_ids":["at-1"]}"#);
    let (updated, payload) = processed(
        Some(&access_token),
        Some(&team_handler("team_ids")),
        None,
        None,
    );
    assert!(updated.is_none());
    assert!(payload.is_none());
}

#[test]
fn the_plural_team_claim_fills_empty_teams() {
    let access_token = token(r#"{"team_ids":["at-1","at-2"],"team_id":"at-3"}"#);
    let (updated, payload) = processed(
        Some(&access_token),
        Some(&team_handler("team_ids")),
        Some(identity()),
        None,
    );
    assert_eq!(updated.unwrap().team_ids, strings(&["at-1", "at-2"]));
    assert_eq!(
        payload,
        Some(document(r#"{"team_ids":["at-1","at-2"],"team_id":"at-3"}"#))
    );
}

#[test]
fn existing_teams_are_kept() {
    let mut with_teams = identity();
    with_teams.team_ids = strings(&["existing"]);
    let access_token = token(r#"{"team_ids":["at-1"]}"#);
    let (updated, _payload) = processed(
        Some(&access_token),
        Some(&team_handler("team_ids")),
        Some(with_teams),
        None,
    );
    assert_eq!(updated.unwrap().team_ids, strings(&["existing"]));
}

#[test]
fn teams_stay_unset_without_an_sso_handler() {
    let access_token = token(r#"{"team_ids":["at-1"]}"#);
    let (updated, payload) = processed(Some(&access_token), None, Some(identity()), None);
    assert!(updated.unwrap().team_ids.is_empty());
    assert!(payload.is_some());
}

#[test]
fn an_existing_role_is_kept() {
    let mut with_role = identity();
    with_role.user_role = Some(UserRole::ProxyAdmin);
    let access_token = token(r#"{"groups":["admins"],"role":"internal_user"}"#);
    let role_mappings = mappings(None, &[(UserRole::ProxyAdminViewer, &["admins"])]);
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(with_role),
        Some(&role_mappings),
    );
    assert_eq!(updated.unwrap().user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn a_group_match_decides_before_the_direct_claim() {
    let access_token = token(r#"{"groups":["admins"],"role":"internal_user"}"#);
    let role_mappings = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(identity()),
        Some(&role_mappings),
    );
    assert_eq!(updated.unwrap().user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn the_default_role_stands_in_when_no_group_matches() {
    let access_token = token(r#"{"groups":["access-group"],"role":"internal_user"}"#);
    let role_mappings = mappings(
        Some(UserRole::InternalUserViewer),
        &[(UserRole::ProxyAdmin, &["admins"])],
    );
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(identity()),
        Some(&role_mappings),
    );
    assert_eq!(
        updated.unwrap().user_role,
        Some(UserRole::InternalUserViewer)
    );
}

#[test]
fn no_group_claim_takes_the_default_role() {
    let access_token = token(r#"{"role":"internal_user"}"#);
    let role_mappings = mappings(
        Some(UserRole::InternalUserViewer),
        &[(UserRole::ProxyAdmin, &["admins"])],
    );
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(identity()),
        Some(&role_mappings),
    );
    assert_eq!(
        updated.unwrap().user_role,
        Some(UserRole::InternalUserViewer)
    );
}

#[test]
fn a_mapping_without_roles_skips_the_default_role() {
    let access_token = token(r#"{"groups":["admins"],"role":"internal_user"}"#);
    let role_mappings = mappings(Some(UserRole::InternalUserViewer), &[]);
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(identity()),
        Some(&role_mappings),
    );
    assert_eq!(updated.unwrap().user_role, Some(UserRole::InternalUser));
}

#[test]
fn a_comma_separated_group_claim_decides_the_role() {
    let access_token = token(r#"{"groups":" staff , admins "}"#);
    let role_mappings = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(identity()),
        Some(&role_mappings),
    );
    assert_eq!(updated.unwrap().user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn a_scalar_group_claim_decides_the_role() {
    let access_token = token(r#"{"groups":"admins"}"#);
    let role_mappings = mappings(None, &[(UserRole::InternalUser, &["admins"])]);
    let (updated, _payload) = processed(
        Some(&access_token),
        None,
        Some(identity()),
        Some(&role_mappings),
    );
    assert_eq!(updated.unwrap().user_role, Some(UserRole::InternalUser));
}

#[test]
fn the_role_claim_list_resolves_to_its_highest_member() {
    let access_token = token(r#"{"role":["internal_user","proxy_admin"]}"#);
    let (updated, _payload) = processed(Some(&access_token), None, Some(identity()), None);
    assert_eq!(updated.unwrap().user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn a_token_without_a_role_leaves_it_unset() {
    let access_token = token(r#"{"team_ids":[]}"#);
    let (updated, _payload) = processed(Some(&access_token), None, Some(identity()), None);
    assert!(updated.unwrap().user_role.is_none());
}

#[test]
fn a_null_role_in_the_token_leaves_it_unset() {
    let access_token = token(r#"{"role":null,"groups":null}"#);
    let (updated, _payload) = processed(Some(&access_token), None, Some(identity()), None);
    assert!(updated.unwrap().user_role.is_none());
}
