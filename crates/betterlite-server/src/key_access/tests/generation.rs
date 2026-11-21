use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::user_roles::UserRole;
use rstest::rstest;

use super::support::{TEAM, caller, member, request, settings, team};
use crate::key_access::error::{DetailShape, KeyAccessError};
use crate::key_access::generation::key_generation_check;
use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;

const OPTED_IN: &str = r#"{"team_id":"team-1","members_with_roles":[{"user_id":"member-1","role":"user"}],"team_member_permissions":["/key/generate","/key/access_group_assignment"]}"#;
const BASELINE: &str =
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"member-1","role":"user"}]}"#;
const ADMIN_ONLY: &str = "litellm_settings:\n  key_generation_settings:\n    team_key_generation:\n      allowed_team_member_roles: [admin]\n";
const NEEDS_ALIAS: &str = "litellm_settings:\n  key_generation_settings:\n    team_key_generation:\n      required_params: [key_alias]\n    personal_key_generation:\n      required_params: [key_alias]\n";
const PERSONAL_ADMINS: &str = "litellm_settings:\n  key_generation_settings:\n    personal_key_generation:\n      allowed_user_roles: [proxy_admin]\n";

fn generate(
    who: &TeamCaller,
    roster: Option<&str>,
    body: &str,
    source: &str,
) -> Result<(), KeyAccessError> {
    let found = roster.map(team);
    key_generation_check(
        who,
        found.as_ref(),
        &request(body),
        KeyManagementRoute::KeyGenerate,
        &settings(source).key_generation,
    )
}

fn proxy_admin() -> TeamCaller {
    caller(Some("admin-9"), Some(UserRole::ProxyAdmin))
}

#[rstest]
#[case::default_team_roles_user(member("member-1"), "{}")]
#[case::default_team_roles_admin(member("admin-1"), "{}")]
#[case::configured_team_roles_missing(
    member("member-1"),
    "litellm_settings:\n  key_generation_settings:\n    team_key_generation: {}\n"
)]
#[case::admin_role_listed(member("admin-1"), ADMIN_ONLY)]
fn team_roles_allowed(#[case] who: TeamCaller, #[case] source: &str) {
    generate(&who, Some(TEAM), r#"{"team_id":"team-1"}"#, source).unwrap();
}

#[test]
fn team_role_disallowed() {
    let error = generate(
        &member("member-1"),
        Some(TEAM),
        r#"{"team_id":"team-1"}"#,
        ADMIN_ONLY,
    )
    .unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::RoleNotAllowed { role, allowed } if role == "user" && allowed == &["admin".to_owned()])
    );
    assert_eq!(error.status(), Some(400_u16));
    assert_eq!(error.detail(), Some(DetailShape::Text));
}

#[test]
fn team_proxy_admin_shortcut() {
    let body = r#"{"team_id":"team-1","user_id":"outsider","access_group_ids":["group-1"]}"#;
    generate(&proxy_admin(), Some(TEAM), body, NEEDS_ALIAS).unwrap();
}

#[rstest]
#[case::explicit_null(r#"{"team_id":"team-1","key_alias":null}"#, true)]
#[case::given(r#"{"team_id":"team-1","key_alias":"alias"}"#, true)]
#[case::absent(r#"{"team_id":"team-1"}"#, false)]
#[case::before_access_groups(r#"{"team_id":"team-1","access_group_ids":["group-1"]}"#, false)]
fn required_fields_presence(#[case] body: &str, #[case] present: bool) {
    let outcome = generate(&member("member-1"), Some(TEAM), body, NEEDS_ALIAS);
    if present {
        outcome.unwrap();
        return;
    }
    let error = outcome.unwrap_err();
    assert!(matches!(&error, KeyAccessError::MissingParam { param } if param == "key_alias"));
    assert_eq!(error.status(), Some(400_u16));
    assert_eq!(error.detail(), Some(DetailShape::Text));
}

#[test]
fn team_endpoint_permission_before_required_fields() {
    let error = generate(
        &member("member-1"),
        Some(BASELINE),
        r#"{"team_id":"team-1"}"#,
        NEEDS_ALIAS,
    )
    .unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::Team(inner) if matches!(**inner, TeamAccessError::MemberRoute { .. }))
    );
}

#[rstest]
#[case::not_opted_in(TEAM, false)]
#[case::opted_in(OPTED_IN, true)]
fn team_access_groups_opt_in(#[case] roster: &str, #[case] allowed: bool) {
    let body = r#"{"team_id":"team-1","access_group_ids":["group-1"]}"#;
    let outcome = generate(&member("member-1"), Some(roster), body, "{}");
    if allowed {
        outcome.unwrap();
        return;
    }
    let error = outcome.unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::Team(inner) if matches!(**inner, TeamAccessError::GroupOptIn { .. }))
    );
    assert_eq!(error.status(), Some(403_u16));
    assert_eq!(error.detail(), Some(DetailShape::Text));
}

#[test]
fn personal_access_groups_first() {
    let error = generate(
        &member("member-1"),
        None,
        r#"{"access_group_ids":["group-1"]}"#,
        PERSONAL_ADMINS,
    )
    .unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::Team(inner) if matches!(**inner, TeamAccessError::PersonalKeyGroups { .. }))
    );
    assert_eq!(error.status(), Some(403_u16));
}

#[rstest]
#[case::unset("{}", member("member-1"), "{}", "allowed")]
#[case::personal_absent(
    "litellm_settings:\n  key_generation_settings: {}\n",
    member("member-1"),
    "{}",
    "allowed"
)]
#[case::roles_absent(
    "litellm_settings:\n  key_generation_settings:\n    personal_key_generation: {}\n",
    member("member-1"),
    "{}",
    "allowed"
)]
#[case::role_listed(
    "litellm_settings:\n  key_generation_settings:\n    personal_key_generation:\n      allowed_user_roles: [internal_user]\n",
    member("member-1"),
    "{}",
    "allowed"
)]
#[case::role_unlisted(PERSONAL_ADMINS, member("member-1"), "{}", "role")]
#[case::role_missing(PERSONAL_ADMINS, caller(Some("member-1"), None), "{}", "role")]
#[case::blank_team_is_personal(PERSONAL_ADMINS, member("member-1"), r#"{"team_id":""}"#, "role")]
#[case::personal_param_missing(NEEDS_ALIAS, member("member-1"), "{}", "param")]
#[case::personal_param_given(
    NEEDS_ALIAS,
    member("member-1"),
    r#"{"key_alias":"alias"}"#,
    "allowed"
)]
fn personal_roles(
    #[case] source: &str,
    #[case] who: TeamCaller,
    #[case] body: &str,
    #[case] expected: &str,
) {
    let kind = match generate(&who, None, body, source) {
        Ok(()) => "allowed",
        Err(KeyAccessError::PersonalRole { .. }) => "role",
        Err(KeyAccessError::MissingParam { .. }) => "param",
        Err(other) => panic!("unexpected {other:?}"),
    };
    assert_eq!(kind, expected);
}

#[rstest]
#[case::admin_with_settings(
    proxy_admin(),
    "litellm_settings:\n  key_generation_settings: {}\n",
    false
)]
#[case::admin_unconfigured(proxy_admin(), "{}", true)]
#[case::member_unconfigured(member("member-1"), "{}", false)]
#[case::member_with_settings(member("member-1"), ADMIN_ONLY, false)]
fn missing_team_settings(#[case] who: TeamCaller, #[case] source: &str, #[case] allowed: bool) {
    let outcome = generate(&who, None, r#"{"team_id":"team-404"}"#, source);
    if allowed {
        outcome.unwrap();
        return;
    }
    let error = outcome.unwrap_err();
    assert!(matches!(&error, KeyAccessError::TeamNotFound { team_id } if team_id == "team-404"));
    assert_eq!(error.status(), Some(400_u16));
    assert_eq!(error.detail(), Some(DetailShape::Text));
}
