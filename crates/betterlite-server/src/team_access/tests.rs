#![cfg(test)]

mod lookups;
mod support;

use std::sync::{Arc, Mutex};

use betterlite_auth::management_routes::KEY_MANAGEMENT_ROUTES;
use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::team::TeamBase;
use rstest::rstest;

use super::access::{TEAM_ADMIN_ONLY, TEAM_OR_ORG_ADMIN, TeamAccess, TeamRole, is_team_admin};
use super::caller::TeamCaller;
use super::error::TeamAccessError;
use super::permissions::{
    BASELINE_PERMISSIONS, enforce_group_assignment, member_permissions, member_route_allowed,
};
use super::roster::{caller_team_role, member_has_permission};
use support::{RecordingOrgRoles, caller, team};

const ORG_TEAM: &str = r#"{"team_id":"team-1","organization_id":"org-1","members_with_roles":[{"user_id":"admin-1","role":"admin"},{"user_id":"member-1","role":"user"}]}"#;

fn recorded_access(
    admins: Vec<(&'static str, &'static str)>,
) -> (Arc<RecordingOrgRoles>, TeamAccess) {
    let roles = Arc::new(RecordingOrgRoles {
        admins,
        lookups: Mutex::default(),
    });
    let access = TeamAccess {
        org_roles: Arc::<RecordingOrgRoles>::clone(&roles),
    };
    (roles, access)
}

fn permissions_team(permissions: &str) -> TeamBase<String> {
    team(&format!(
        r#"{{"team_id":"team-1","members_with_roles":[{{"user_id":"admin-1","role":"admin"}},{{"user_id":"member-1","role":"user"}}],"team_member_permissions":{permissions}}}"#
    ))
}

fn values(routes: &[KeyManagementRoute]) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = routes
        .iter()
        .copied()
        .map(KeyManagementRoute::value)
        .collect();
    found.sort_unstable();
    found
}

fn auth_caller(text: &str) -> TeamCaller {
    let JsonData::Object(fields) = serde_json::from_str(text).unwrap() else {
        panic!("a caller fixture is an object");
    };
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    TeamCaller::try_from(&auth).unwrap()
}

#[rstest]
#[case::admin(UserRole::ProxyAdmin, true, Some(TeamRole::ProxyAdmin))]
#[case::viewer(UserRole::ProxyAdminViewer, false, None)]
#[tokio::test]
async fn proxy_admin(
    #[case] role: UserRole,
    #[case] allowed: bool,
    #[case] strongest: Option<TeamRole>,
) {
    let (_, access) = recorded_access(Vec::new());
    let outsider = caller(Some("outsider"), Some(role));
    let base = team(ORG_TEAM);
    assert_eq!(
        access
            .allows(&outsider, &base, &TEAM_ADMIN_ONLY)
            .await
            .unwrap(),
        allowed
    );
    assert_eq!(
        access
            .allows(&outsider, &base, &TEAM_OR_ORG_ADMIN)
            .await
            .unwrap(),
        allowed
    );
    assert_eq!(
        access.strongest_role(&outsider, &base).await.unwrap(),
        strongest
    );
}

#[tokio::test]
async fn team_admin() {
    let (roles, access) = recorded_access(Vec::new());
    let base = team(ORG_TEAM);
    let admin = caller(Some("admin-1"), None);
    assert!(
        access
            .allows(&admin, &base, &TEAM_ADMIN_ONLY)
            .await
            .unwrap()
    );
    assert!(
        !access
            .allows(&caller(Some("member-1"), None), &base, &TEAM_ADMIN_ONLY)
            .await
            .unwrap()
    );
    assert!(roles.lookups.lock().unwrap().is_empty());
    assert_eq!(
        access.strongest_role(&admin, &base).await.unwrap(),
        Some(TeamRole::TeamAdmin)
    );
    let by_email = team(
        r#"{"team_id":"team-1","members_with_roles":[{"user_email":"a@x.io","role":"admin"}]}"#,
    );
    assert!(!is_team_admin(&caller(Some("a@x.io"), None), &by_email));
}

#[test]
fn member_key_route() {
    let base = permissions_team(r#"["/key/generate","/key/{key_id}/regenerate"]"#);
    let member = Some(&MemberRole::User);
    assert!(member_route_allowed(member, &base, "/key/generate").unwrap());
    assert!(member_route_allowed(member, &base, "/key/health").unwrap());
    assert!(member_route_allowed(member, &base, "/key/sk-1/regenerate").unwrap());
}

#[tokio::test]
async fn allows_short_circuit() {
    let (roles, access) = recorded_access(Vec::new());
    let base = team(ORG_TEAM);
    let proxy = caller(Some("outsider"), Some(UserRole::ProxyAdmin));
    let roster_and_org = [TeamRole::TeamAdmin, TeamRole::OrgAdmin];
    assert!(!access.allows(&proxy, &base, &roster_and_org).await.unwrap());
    assert_eq!(roles.lookups.lock().unwrap().len(), 1);
    let admin = caller(Some("admin-1"), None);
    assert!(
        access
            .allows(&admin, &base, &TEAM_OR_ORG_ADMIN)
            .await
            .unwrap()
    );
    assert!(
        !access
            .allows(&caller(Some("member-1"), None), &base, &TEAM_ADMIN_ONLY)
            .await
            .unwrap()
    );
    assert_eq!(roles.lookups.lock().unwrap().len(), 1);
    assert!(
        !access
            .allows(&admin, &base, &[TeamRole::OrgAdmin])
            .await
            .unwrap()
    );
    assert_eq!(roles.lookups.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn strongest_org_before_team() {
    let (roles, access) = recorded_access(vec![("admin-1", "org-1")]);
    let base = team(ORG_TEAM);
    let admin = caller(Some("admin-1"), None);
    assert_eq!(
        access.strongest_role(&admin, &base).await.unwrap(),
        Some(TeamRole::OrgAdmin)
    );
    assert_eq!(
        *roles.lookups.lock().unwrap(),
        vec![("admin-1".to_owned(), "org-1".to_owned())]
    );
}

#[rstest]
#[case::no_user(None, Some("org-1"))]
#[case::empty_user(Some(""), Some("org-1"))]
#[case::no_organization(Some("org-user"), None)]
#[case::empty_organization(Some("org-user"), Some(""))]
#[tokio::test]
async fn missing_org_identity(#[case] user_id: Option<&str>, #[case] organization: Option<&str>) {
    let (roles, access) = recorded_access(vec![("", "org-1"), ("org-user", "")]);
    let mut base = team(ORG_TEAM);
    base.organization_id = organization.map(str::to_owned);
    let outsider = caller(user_id, None);
    assert!(
        !access
            .allows(&outsider, &base, &TEAM_OR_ORG_ADMIN)
            .await
            .unwrap()
    );
    assert_eq!(access.strongest_role(&outsider, &base).await.unwrap(), None);
    assert!(roles.lookups.lock().unwrap().is_empty());
}

#[test]
fn null_roster_id() {
    let base = team(
        r#"{"team_id":"team-1","members_with_roles":[{"user_email":"e@x.io","role":"admin"}],"team_member_permissions":["/key/generate"]}"#,
    );
    let anonymous = caller(None, None);
    assert!(!is_team_admin(&anonymous, &base));
    assert_eq!(caller_team_role(&base, &anonymous), None);
    assert!(!member_has_permission(&anonymous, &base, "/key/generate"));
}

#[rstest]
#[case::user_first(
    r#"[{"user_id":"dup","role":"user"},{"user_id":"dup","role":"admin"}]"#,
    MemberRole::User,
    true
)]
#[case::admin_first(
    r#"[{"user_id":"dup","role":"admin"},{"user_id":"dup","role":"user"}]"#,
    MemberRole::Admin,
    true
)]
#[case::null_id_first(
    r#"[{"user_email":"dup","role":"admin"},{"user_id":"dup","role":"user"}]"#,
    MemberRole::User,
    false
)]
fn first_roster_match(#[case] roster: &str, #[case] role: MemberRole, #[case] admin: bool) {
    let base = team(&format!(
        r#"{{"team_id":"team-1","members_with_roles":{roster}}}"#
    ));
    let duplicate = caller(Some("dup"), None);
    assert_eq!(caller_team_role(&base, &duplicate), Some(role));
    assert_eq!(is_team_admin(&duplicate, &base), admin);
}

#[test]
fn service_account_user_role() {
    let account = auth_caller(r#"{"team_id":"team-1","metadata":{"service_account_id":"sa-1"}}"#);
    assert!(account.team_service_account);
    assert_eq!(
        caller_team_role(&team(ORG_TEAM), &account),
        Some(MemberRole::User)
    );
    let person = auth_caller(
        r#"{"user_id":"admin-1","user_role":"proxy_admin","team_id":"team-1","metadata":{"service_account_id":"sa-1"}}"#,
    );
    assert!(!person.team_service_account);
    assert_eq!(person.user_role, Some(UserRole::ProxyAdmin));
    assert_eq!(
        caller_team_role(&team(ORG_TEAM), &person),
        Some(MemberRole::Admin)
    );
}

#[test]
fn foreign_service_account() {
    let account = auth_caller(r#"{"team_id":"team-2","metadata":{"service_account_id":"sa-1"}}"#);
    let base = permissions_team(r#"["/key/access_group_assignment"]"#);
    assert_eq!(caller_team_role(&base, &account), None);
    let groups = ["group-1".to_owned()];
    let denied = enforce_group_assignment(&account, Some(&base), Some(&groups)).unwrap_err();
    assert!(matches!(denied, TeamAccessError::GroupOptIn { .. }));
}

#[test]
fn baseline_none() {
    let base = team(r#"{"team_id":"team-1","team_member_permissions":null}"#);
    assert_eq!(
        member_permissions(&base).unwrap(),
        BASELINE_PERMISSIONS.to_vec()
    );
}

#[test]
fn baseline_empty() {
    assert_eq!(
        values(&member_permissions(&permissions_team("[]")).unwrap()),
        ["/key/health", "/key/info"]
    );
}

#[test]
fn permissions_deduplicated() {
    let base = permissions_team(r#"["/key/generate","/key/generate","/key/info"]"#);
    assert_eq!(
        values(&member_permissions(&base).unwrap()),
        ["/key/generate", "/key/health", "/key/info"]
    );
}

#[test]
fn unknown_permission_error() {
    let base = permissions_team(r#"["/key/generate","/key/*"]"#);
    let error = member_permissions(&base).unwrap_err();
    assert!(matches!(&error, TeamAccessError::UnknownPermission { value } if value == "/key/*"));
    assert_eq!(error.status(), None);
    member_route_allowed(Some(&MemberRole::User), &base, "/key/generate").unwrap_err();
}

// a string where LiteLLM's model declares list[str] | None fails decoding, so the isinstance fallback can't be reached
#[test]
fn nonlist_baseline() {
    let decoded = serde_json::from_str::<TeamBase<String>>(
        r#"{"team_id":"team-1","team_member_permissions":"/key/generate"}"#,
    );
    assert!(decoded.unwrap_err().to_string().contains("invalid type"));
}

#[test]
fn member_route_401() {
    let base = permissions_team(r#"["/key/info"]"#);
    let error = member_route_allowed(Some(&MemberRole::User), &base, "/key/generate").unwrap_err();
    assert_eq!(error.status(), Some(401));
    let exception = error.proxy_exception().unwrap();
    assert_eq!(exception.error_type, "team_member_permission_error");
    assert_eq!(exception.param.as_deref(), Some("/key/generate"));
    assert_eq!(exception.code, "401");
}

#[test]
fn admin_route_bypass() {
    let base = permissions_team(r#"["/key/*"]"#);
    assert!(member_route_allowed(Some(&MemberRole::Admin), &base, "/key/delete").unwrap());
    assert!(!member_route_allowed(None, &base, "/key/delete").unwrap());
}

#[test]
fn explicit_permission_only() {
    let base = permissions_team(r#"["/key/generate"]"#);
    let member = caller(Some("member-1"), None);
    assert!(member_has_permission(&member, &base, "/key/generate"));
    assert!(!member_has_permission(&member, &base, "/key/info"));
    assert!(!member_has_permission(
        &caller(Some("outsider"), None),
        &base,
        "/key/generate"
    ));
    let account = auth_caller(r#"{"team_id":"team-1","metadata":{"service_account_id":"sa-1"}}"#);
    assert!(!member_has_permission(&account, &base, "/key/generate"));
    assert!(!member_has_permission(
        &member,
        &permissions_team("[]"),
        "/key/generate"
    ));
    assert!(!member_has_permission(
        &member,
        &permissions_team("null"),
        "/key/generate"
    ));
}

#[test]
fn all_routes_order() {
    assert_eq!(KEY_MANAGEMENT_ROUTES.len(), 24);
    assert_eq!(KEY_MANAGEMENT_ROUTES.first(), Some(&"/key/generate"));
    assert_eq!(KEY_MANAGEMENT_ROUTES.get(3), Some(&"/key/info"));
    assert_eq!(KEY_MANAGEMENT_ROUTES.last(), Some(&"/auto_router/manage"));
    assert!(!KEY_MANAGEMENT_ROUTES.contains(&"/key/health"));
}

#[test]
fn default_routes() {
    let defaults: Vec<&str> = BASELINE_PERMISSIONS
        .into_iter()
        .map(KeyManagementRoute::value)
        .collect();
    assert_eq!(defaults, ["/key/info", "/key/health"]);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(Vec::new()))]
fn empty_groups_noop(#[case] groups: Option<Vec<String>>) {
    let member = caller(Some("member-1"), None);
    enforce_group_assignment(&member, None, groups.as_deref()).unwrap();
}

#[test]
fn proxy_group_bypass() {
    let proxy = caller(Some("outsider"), Some(UserRole::ProxyAdmin));
    let groups = ["group-1".to_owned()];
    enforce_group_assignment(&proxy, None, Some(&groups)).unwrap();
    let viewer = caller(Some("outsider"), Some(UserRole::ProxyAdminViewer));
    enforce_group_assignment(&viewer, None, Some(&groups)).unwrap_err();
}

#[test]
fn team_admin_group_bypass() {
    let groups = ["group-1".to_owned()];
    let admin = caller(Some("admin-1"), None);
    enforce_group_assignment(
        &admin,
        Some(&permissions_team(r#"["/key/*"]"#)),
        Some(&groups),
    )
    .unwrap();
}

#[test]
fn personal_group_403() {
    let groups = ["group-b".to_owned(), "group-a".to_owned()];
    let error =
        enforce_group_assignment(&caller(Some("member-1"), None), None, Some(&groups)).unwrap_err();
    assert!(
        matches!(&error, TeamAccessError::PersonalKeyGroups { denied } if *denied == ["group-a", "group-b"])
    );
    assert_eq!(error.status(), Some(403));
    assert!(error.proxy_exception().is_none());
}

#[rstest]
#[case::opted_in(r#"["/key/access_group_assignment"]"#, true)]
#[case::other_route(r#"["/key/generate"]"#, false)]
#[case::empty("[]", false)]
#[case::absent("null", false)]
fn member_group_opt_in(#[case] permissions: &str, #[case] allowed: bool) {
    let groups = ["group-1".to_owned()];
    let member = caller(Some("member-1"), None);
    let outcome =
        enforce_group_assignment(&member, Some(&permissions_team(permissions)), Some(&groups));
    assert_eq!(outcome.is_ok(), allowed);
    if let Err(error) = outcome {
        assert!(matches!(&error, TeamAccessError::GroupOptIn { team_id } if team_id == "team-1"));
        assert_eq!(error.status(), Some(403));
    }
}

#[test]
fn outsider_group_403() {
    let groups = ["group-1".to_owned()];
    let base = permissions_team(r#"["/key/access_group_assignment"]"#);
    let error =
        enforce_group_assignment(&caller(Some("outsider"), None), Some(&base), Some(&groups))
            .unwrap_err();
    assert!(matches!(error, TeamAccessError::GroupOptIn { .. }));
}

#[test]
fn wildcard_not_opt_in() {
    let groups = ["group-1".to_owned()];
    let member = caller(Some("member-1"), None);
    let template = permissions_team(r#"["/key/{key_id}/regenerate"]"#);
    let refused = enforce_group_assignment(&member, Some(&template), Some(&groups)).unwrap_err();
    assert!(matches!(refused, TeamAccessError::GroupOptIn { .. }));
    let wildcard = permissions_team(r#"["/key/*"]"#);
    let invalid = enforce_group_assignment(&member, Some(&wildcard), Some(&groups)).unwrap_err();
    assert!(matches!(invalid, TeamAccessError::UnknownPermission { .. }));
}

#[test]
fn team_denied_403() {
    let denied = TeamAccessError::Denied;
    assert_eq!(denied.status(), Some(403));
    assert_eq!(denied.to_string(), "You do not have access to this team");
    assert!(denied.proxy_exception().is_none());
}
