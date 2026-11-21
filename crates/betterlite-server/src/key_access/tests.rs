#![cfg(test)]

mod generation;
mod postgres;
mod support;

use std::sync::Mutex;

use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::identity_rows::Payload;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_logging::callback_config::error::{PayloadError, ValidationError};
use rstest::rstest;
use secrecy::SecretString;

use super::common::common_key_access_checks;
use super::custom_key::check_custom_key_allowed;
use super::error::{DetailShape, KeyAccessError};
use super::generation::key_generation_check;
use super::logging_metadata::check_key_logging_config;
use super::service_account::{TeamDirectory, validate_service_account_team};
use super::team_member::team_member_check;
use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;
use support::{ListedTeams, TEAM, caller, member, request, settings, team, warned};

const WARNING: &str = "Custom API key rejected: disable_custom_api_keys is enabled";

#[rstest]
#[case::empty_key("")]
#[case::custom_key("sk-custom-value-0001")]
fn custom_key_disabled(#[case] key: &str) {
    let disabled = settings("general_settings:\n  disable_custom_api_keys: 'True'\n");
    let secret = SecretString::from(key);
    let (outcome, warnings) = warned(|| check_custom_key_allowed(Some(&secret), &disabled));
    let error = outcome.unwrap_err();
    assert!(matches!(error, KeyAccessError::CustomKeyDisabled));
    assert_eq!(error.status(), Some(403_u16));
    assert_eq!(error.detail(), Some(DetailShape::Error));
    assert_eq!(warnings, vec![WARNING.to_owned()]);
}

#[rstest]
#[case::absent_key_bypasses("general_settings:\n  disable_custom_api_keys: true\n", None)]
#[case::padded_text_is_false(
    "general_settings:\n  disable_custom_api_keys: ' true '\n",
    Some("sk-custom-value-0001")
)]
#[case::unset_switch("{}", Some(""))]
fn custom_key_allowed(#[case] source: &str, #[case] key: Option<&str>) {
    let switch = settings(source);
    let secret = key.map(SecretString::from);
    let (outcome, warnings) = warned(|| check_custom_key_allowed(secret.as_ref(), &switch));
    outcome.unwrap();
    assert!(warnings.is_empty());
}

#[rstest]
#[case::admin_for_another_user(
    caller(Some("admin-1"), Some(UserRole::ProxyAdmin)),
    r#"{"user_id":"someone"}"#,
    None,
    true
)]
#[case::own_key(member("member-1"), r#"{"user_id":"member-1"}"#, None, true)]
#[case::absent_user_id(member("member-1"), "{}", None, true)]
#[case::another_user(member("member-1"), r#"{"user_id":"someone"}"#, None, false)]
#[case::caller_without_id(
    caller(None, Some(UserRole::InternalUser)),
    r#"{"user_id":"someone"}"#,
    None,
    false
)]
#[case::viewer_admin(
    caller(Some("viewer"), Some(UserRole::ProxyAdminViewer)),
    r#"{"user_id":"someone"}"#,
    None,
    false
)]
#[case::override_wins(member("member-1"), r#"{"user_id":"someone"}"#, Some("member-1"), true)]
#[case::override_checked(
    member("member-1"),
    r#"{"user_id":"member-1"}"#,
    Some("someone"),
    false
)]
fn personal_key_owner(
    #[case] who: TeamCaller,
    #[case] body: &str,
    #[case] user_id: Option<&str>,
    #[case] allowed: bool,
) {
    let outcome = common_key_access_checks(&who, &request(body).base, user_id);
    if allowed {
        outcome.unwrap();
        return;
    }
    let error = outcome.unwrap_err();
    assert!(matches!(error, KeyAccessError::NotOwner { .. }));
    assert_eq!(error.status(), Some(403_u16));
    assert_eq!(error.detail(), Some(DetailShape::Text));
}

#[test]
fn dashboard_team_keeps_owner_check() {
    let dashboard = TeamCaller {
        user_id: Some("member-1".to_owned()),
        user_role: Some(UserRole::InternalUser),
        team_id: Some("litellm-dashboard".to_owned()),
        team_service_account: false,
    };
    let data = request(r#"{"user_id":"someone","team_id":"team-1"}"#);
    let error = common_key_access_checks(&dashboard, &data.base, None).unwrap_err();
    assert!(matches!(error, KeyAccessError::NotOwner { user_id } if user_id == "someone"));
}

#[test]
fn model_access_group_without_license() {
    let data = request(r#"{"user_id":"member-1","models":["wildcard-access-group","openai/*"]}"#);
    common_key_access_checks(&member("member-1"), &data.base, None).unwrap();
}

#[rstest]
#[case::member_route_granted(TEAM, "member-1", true)]
#[case::admin_skips_permissions(r#"{"team_id":"team-1","members_with_roles":[{"user_id":"admin-1","role":"admin"}],"team_member_permissions":[]}"#, "admin-1", true)]
#[case::baseline_only(
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"member-1","role":"user"}]}"#,
    "member-1",
    false
)]
fn team_member_permission(#[case] roster: &str, #[case] user_id: &str, #[case] allowed: bool) {
    let config = settings("{}").key_generation;
    let data = request(r#"{"team_id":"team-1"}"#);
    let found = team(roster);
    let outcome = key_generation_check(
        &member(user_id),
        Some(&found),
        &data,
        KeyManagementRoute::KeyGenerate,
        &config,
    );
    if allowed {
        outcome.unwrap();
        return;
    }
    let error = outcome.unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::Team(inner) if matches!(**inner, TeamAccessError::MemberRoute { .. }))
    );
    assert_eq!(error.status(), Some(401_u16));
    assert_eq!(error.detail(), None);
    let exception = error.proxy_exception().unwrap();
    assert_eq!(exception.error_type, "team_member_permission_error");
    assert_eq!(exception.param.as_deref(), Some("/key/generate"));
    assert_eq!(exception.code, "401");
}

#[tokio::test]
async fn service_account_team_required() {
    let directory = ListedTeams {
        teams: vec!["team-1"],
        asked: Mutex::new(Vec::new()),
    };
    let teams: Option<&dyn TeamDirectory> = Some(&directory);
    let required = validate_service_account_team(None, teams)
        .await
        .unwrap_err();
    assert!(matches!(required, KeyAccessError::ServiceTeamRequired));
    let database = validate_service_account_team(Some("team-1"), None)
        .await
        .unwrap_err();
    assert!(matches!(database, KeyAccessError::ServiceDatabaseRequired));
    let unknown = validate_service_account_team(Some("team-2"), teams)
        .await
        .unwrap_err();
    assert!(
        matches!(&unknown, KeyAccessError::ServiceTeamUnknown { team_id } if team_id == "team-2")
    );
    validate_service_account_team(Some("team-1"), teams)
        .await
        .unwrap();
    let shapes = [required, database, unknown].map(|error| (error.status(), error.detail()));
    assert_eq!(shapes, [(Some(400_u16), Some(DetailShape::Text)); 3]);
    assert_eq!(*directory.asked.lock().unwrap(), vec!["team-2", "team-1"]);
}

#[rstest]
#[case::other_team(r#"{"team_id":"team-2"}"#)]
#[case::personal_key("{}")]
fn service_account_cross_team(#[case] body: &str) {
    let account = TeamCaller {
        user_id: None,
        user_role: Some(UserRole::ProxyAdmin),
        team_id: Some("team-1".to_owned()),
        team_service_account: true,
    };
    let config = settings("{}").key_generation;
    let found = team(TEAM);
    let error = key_generation_check(
        &account,
        Some(&found),
        &request(body),
        KeyManagementRoute::KeyGenerate,
        &config,
    )
    .unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::ServiceAccountTeam { team_id } if team_id.as_deref() == Some("team-1"))
    );
    assert_eq!(error.status(), Some(403_u16));
    key_generation_check(
        &account,
        Some(&found),
        &request(r#"{"team_id":"team-1"}"#),
        KeyManagementRoute::KeyGenerate,
        &config,
    )
    .unwrap();
}

#[rstest]
#[case::admin_with_assigned_outsider(
    caller(Some("admin-9"), Some(UserRole::ProxyAdmin)),
    Some("outsider"),
    "assigned"
)]
#[case::admin_off_roster(
    caller(Some("admin-9"), Some(UserRole::ProxyAdmin)),
    Some("member-1"),
    "allowed"
)]
#[case::outsider_caller(member("outsider"), Some("member-1"), "caller")]
#[case::assigned_before_caller(member("outsider"), Some("outsider-2"), "assigned")]
#[case::member_for_self(member("member-1"), None, "allowed")]
fn assigned_user_before_admin(
    #[case] who: TeamCaller,
    #[case] assigned: Option<&str>,
    #[case] expected: &str,
) {
    let found = team(TEAM);
    let config = settings("{}").key_generation.team;
    let outcome = team_member_check(
        assigned,
        &found,
        &who,
        &config,
        KeyManagementRoute::KeyGenerate,
    );
    let kind = match outcome {
        Ok(()) => "allowed",
        Err(KeyAccessError::AssignedNotMember { .. }) => "assigned",
        Err(KeyAccessError::CallerNotMember { .. }) => "caller",
        Err(other) => panic!("unexpected {other:?}"),
    };
    assert_eq!(kind, expected);
}

#[test]
fn logging_metadata_rejected() {
    let flags = FeatureFlags {
        otel_v2_enabled: false,
    };
    let scoped: Payload = serde_json::from_str(r#"{"logging":[{"callback_name":"langfuse","callback_vars":{"langfuse_span_scope":"full"}}]}"#).unwrap();
    let error = check_key_logging_config(Some(&scoped), flags).unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::LoggingMetadata(inner) if matches!(**inner, PayloadError::Rejected(ValidationError::SpanScopeCallback { .. })))
    );
    assert_eq!(error.status(), Some(400_u16));
    assert_eq!(error.detail(), Some(DetailShape::Error));
    let newrelic: Payload = serde_json::from_str(r#"{"logging":[{"callback_name":"newrelic","callback_vars":{"newrelic_api_key":"nr-key"}}]}"#).unwrap();
    let flagged = check_key_logging_config(Some(&newrelic), flags).unwrap_err();
    assert!(
        matches!(&flagged, KeyAccessError::LoggingMetadata(inner) if matches!(**inner, PayloadError::Rejected(ValidationError::NewRelicRequiresOtelV2)))
    );
    check_key_logging_config(
        Some(&newrelic),
        FeatureFlags {
            otel_v2_enabled: true,
        },
    )
    .unwrap();
}

#[test]
fn logging_metadata_list_variable_rejected() {
    let listed: Payload = serde_json::from_str(
        r#"{"logging":[{"callback_name":"langfuse","callback_vars":{"langfuse_tags":["a"]}}]}"#,
    )
    .unwrap();
    let flags = FeatureFlags {
        otel_v2_enabled: false,
    };
    let error = check_key_logging_config(Some(&listed), flags).unwrap_err();
    assert!(
        matches!(&error, KeyAccessError::LoggingMetadata(inner) if matches!(**inner, PayloadError::Parse(_)))
    );
    assert_eq!(error.status(), Some(400_u16));
    assert_eq!(error.detail(), Some(DetailShape::Error));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some("{}"))]
#[case::no_logging(Some(r#"{"tags":["a"]}"#))]
#[case::valid_entry(Some(r#"{"logging":[{"callback_name":"langfuse_otel","callback_vars":{"langfuse_span_scope":"llm_only"}}]}"#))]
fn logging_metadata_valid(#[case] metadata: Option<&str>) {
    let payload: Option<Payload> = metadata.map(|text| serde_json::from_str(text).unwrap());
    check_key_logging_config(
        payload.as_ref(),
        FeatureFlags {
            otel_v2_enabled: false,
        },
    )
    .unwrap();
}
