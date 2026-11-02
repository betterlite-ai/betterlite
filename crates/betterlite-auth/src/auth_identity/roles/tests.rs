#![cfg(test)]

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use rstest::rstest;

use super::{
    AuthMethod, PrincipalType, Role, TeamRole, effective_user_role, get_user_role,
    is_user_proxy_admin, map_role, team_role,
};
use crate::user_roles::UserRole;

fn user_with_role(role: Option<&str>) -> LiteLLMUserTable {
    let text = role.map_or_else(
        || r#"{"user_id":"u1"}"#.to_owned(),
        |value| format!(r#"{{"user_id":"u1","user_role":"{value}"}}"#),
    );
    serde_json::from_str(&text).unwrap()
}

#[rstest]
#[case::api_key(AuthMethod::ApiKey, "api_key")]
#[case::http_basic(AuthMethod::HttpBasic, "http_basic")]
#[case::bearer_jwt(AuthMethod::BearerJwt, "bearer_jwt")]
#[case::oauth2_introspection(AuthMethod::Oauth2Introspection, "oauth2_introspection")]
#[case::oidc(AuthMethod::Oidc, "oidc")]
#[case::saml(AuthMethod::Saml, "saml")]
#[case::mutual_tls(AuthMethod::MutualTls, "mutual_tls")]
fn all_auth_method_values(#[case] method: AuthMethod, #[case] value: &str) {
    assert_eq!(method.to_string(), value);
    assert_eq!(
        serde_json::to_string(&method).unwrap(),
        format!("{value:?}")
    );
}

#[rstest]
#[case::human(PrincipalType::Human, "human")]
#[case::service_account(PrincipalType::ServiceAccount, "service_account")]
fn principal_type_values(#[case] principal_type: PrincipalType, #[case] value: &str) {
    assert_eq!(principal_type.to_string(), value);
    assert_eq!(
        serde_json::to_string(&principal_type).unwrap(),
        format!("{value:?}")
    );
}

#[rstest]
#[case::platform_admin(Role::PlatformAdmin, "platform_admin")]
#[case::platform_viewer(Role::PlatformViewer, "platform_viewer")]
#[case::org_admin(Role::OrgAdmin, "org_admin")]
#[case::org_viewer(Role::OrgViewer, "org_viewer")]
#[case::team_admin(Role::TeamAdmin, "team_admin")]
#[case::team_member(Role::TeamMember, "team_member")]
#[case::team_role_admin(TeamRole::Admin, "admin")]
#[case::team_role_member(TeamRole::Member, "member")]
fn normalized_role_values<R>(#[case] role: R, #[case] value: &str)
where
    R: ToString + serde::Serialize,
{
    assert_eq!(role.to_string(), value);
    assert_eq!(serde_json::to_string(&role).unwrap(), format!("{value:?}"));
}

#[rstest]
#[case::proxy_admin("proxy_admin", Role::PlatformAdmin)]
#[case::proxy_admin_viewer("proxy_admin_viewer", Role::PlatformViewer)]
#[case::org_admin("org_admin", Role::OrgAdmin)]
fn map_role_three_legacy_values(#[case] value: &str, #[case] expected: Role) {
    assert_eq!(map_role(Some(value)), Some(expected));
}

#[rstest]
#[case::none(None)]
#[case::internal_user(Some("internal_user"))]
#[case::org_viewer(Some("org_viewer"))]
#[case::team_admin(Some("team_admin"))]
#[case::unknown(Some("not_a_role"))]
fn map_role_unknown_and_none(#[case] value: Option<&str>) {
    assert_eq!(map_role(value), None);
}

#[test]
fn map_role_no_trim_or_casefold() {
    assert_eq!(map_role(Some(" proxy_admin")), None);
    assert_eq!(map_role(Some("proxy_admin ")), None);
    assert_eq!(map_role(Some("PROXY_ADMIN")), None);
}

#[test]
fn team_role_only_exact_admin() {
    assert_eq!(team_role(Some("admin")), TeamRole::Admin);
    assert_eq!(team_role(Some("Admin")), TeamRole::Member);
    assert_eq!(team_role(Some("owner")), TeamRole::Member);
    assert_eq!(team_role(None), TeamRole::Member);
}

#[rstest]
#[case::proxy_admin("proxy_admin", UserRole::ProxyAdmin)]
#[case::proxy_admin_viewer("proxy_admin_viewer", UserRole::ProxyAdminViewer)]
#[case::org_admin("org_admin", UserRole::OrgAdmin)]
#[case::internal_user("internal_user", UserRole::InternalUser)]
#[case::internal_user_viewer("internal_user_viewer", UserRole::InternalUserViewer)]
#[case::team("team", UserRole::Team)]
#[case::customer("customer", UserRole::Customer)]
fn effective_role_seven_values(#[case] value: &str, #[case] expected: UserRole) {
    assert_eq!(effective_user_role(Some(value)), expected);
}

#[rstest]
#[case::none(None)]
#[case::unknown(Some("not_a_role"))]
fn effective_role_invalid_is_internal(#[case] value: Option<&str>) {
    assert_eq!(effective_user_role(value), UserRole::InternalUser);
}

#[test]
fn absent_user_role_is_none() {
    let absent: Option<&LiteLLMUserTable> = None;
    assert_eq!(get_user_role(absent), None);
}

#[rstest]
#[case::proxy_admin(Some("proxy_admin"), Some(UserRole::ProxyAdmin))]
#[case::org_admin(Some("org_admin"), Some(UserRole::OrgAdmin))]
#[case::unknown(Some("not_a_role"), Some(UserRole::InternalUser))]
fn present_user_role_applies_fallback(
    #[case] role: Option<&str>,
    #[case] expected: Option<UserRole>,
) {
    let user = user_with_role(role);
    assert_eq!(get_user_role(Some(&user)), expected);
}

#[rstest]
#[case::proxy_admin(Some("proxy_admin"), true)]
#[case::wrong_case(Some("Proxy_Admin"), false)]
#[case::other_role(Some("org_admin"), false)]
fn proxy_admin_requires_exact_present_role(#[case] role: Option<&str>, #[case] expected: bool) {
    let user = user_with_role(role);
    assert_eq!(is_user_proxy_admin(Some(&user)), expected);
}

#[test]
fn absent_user_is_never_proxy_admin() {
    let absent: Option<&LiteLLMUserTable> = None;
    assert!(!is_user_proxy_admin(absent));
}
