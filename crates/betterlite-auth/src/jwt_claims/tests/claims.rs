use rstest::rstest;

use crate::jwt_claims::claims::ClaimError;
use crate::jwt_claims::identity::IdentityClaims as _;
use crate::jwt_claims::orgs::OrgClaims as _;
use crate::jwt_claims::settings::JwtAuthSettings;
use crate::jwt_claims::teams::TeamClaims as _;
use crate::jwt_claims::trust::TrustClaims as _;

use super::support::{empty_settings, issuers, project};

#[rstest]
#[case::nested(
    "realm.team.id",
    r#"{"realm":{"team":{"id":"team-1"}}}"#,
    Some("fallback"),
    Some("team-1")
)]
#[case::escaped_dot(
    r"kubernetes\.io.tenant",
    r#"{"kubernetes.io":{"tenant":"team-2"}}"#,
    None,
    Some("team-2")
)]
#[case::metadata_prefix("metadata.azp", r#"{"azp":"team-3"}"#, None, Some("team-3"))]
#[case::missing_path("realm.team.id", r#"{"realm":{}}"#, Some("fallback"), Some("fallback"))]
#[case::list_is_not_indexed(
    "groups.0",
    r#"{"groups":["team-4"]}"#,
    Some("fallback"),
    Some("fallback")
)]
#[case::empty_path("", r#"{"": "team-5"}"#, Some("fallback"), Some("fallback"))]
#[case::found_null("realm.team", r#"{"realm":{"team":null}}"#, None, None)]
fn nested_claim_path(
    #[case] field: &str,
    #[case] claims: &str,
    #[case] default: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut settings = empty_settings();
    settings.team_id_jwt_field = Some(field.to_owned());
    let projected = project(claims, &settings, &[]);
    assert_eq!(
        projected.team_id(default).unwrap(),
        expected.map(str::to_owned)
    );
}

#[test]
fn a_non_string_claim_is_a_typed_error() {
    let mut settings = empty_settings();
    settings.user_id_jwt_field = Some("sub".to_owned());
    let projected = project(r#"{"sub":5}"#, &settings, &[]);
    assert!(matches!(
        projected.user_id(None),
        Err(ClaimError::Type { claim, .. }) if claim == "sub"
    ));
}

#[test]
fn a_claim_path_over_the_limit_is_a_typed_error() {
    let mut settings = empty_settings();
    settings.user_id_jwt_field = Some("a".repeat(9000));
    let projected = project(r#"{"sub":"user-1"}"#, &settings, &[]);
    assert!(matches!(
        projected.user_id(None),
        Err(ClaimError::Path { .. })
    ));
}

const ISSUER: &str = "https://idp.example";

fn marker_settings() -> JwtAuthSettings {
    let mut settings = empty_settings();
    settings.user_id_jwt_field = Some("sub".to_owned());
    settings.team_id_jwt_field = Some("team".to_owned());
    settings.team_ids_jwt_field = Some("teams".to_owned());
    settings.user_email_jwt_field = Some("email".to_owned());
    settings.org_id_jwt_field = Some("org".to_owned());
    settings.end_user_id_jwt_field = Some("eid".to_owned());
    settings
}

const TRUSTED_CLAIMS: &str = r#"{
    "_litellm_jwt_issuer": "https://idp.example",
    "_litellm_user_id": "trusted-user",
    "_litellm_team_id": "trusted-team",
    "_litellm_team_ids": "trusted-plural",
    "_litellm_user_email": "trusted@example.com",
    "_litellm_org_id": "trusted-org",
    "_litellm_end_user_id": "trusted-end",
    "sub": "raw-user",
    "team": "raw-team",
    "teams": ["raw-plural"],
    "email": "raw@example.com",
    "org": "raw-org",
    "eid": "raw-end"
}"#;

const RAW_CLAIMS: &str = r#"{
    "_litellm_user_id": "trusted-user",
    "_litellm_team_id": "trusted-team",
    "sub": "raw-user",
    "team": "raw-team"
}"#;

#[test]
fn untrusted_normalization_marker() {
    let settings = marker_settings();
    let configured = issuers(&[ISSUER]);
    let marked = project(TRUSTED_CLAIMS, &settings, &configured);
    assert!(marked.is_trusted_marker_token());
    assert!(marked.claims.json.get("_litellm_team_id").is_some());
    assert_eq!(
        marked.user_id(None).unwrap(),
        Some("trusted-user".to_owned())
    );
    assert_eq!(
        marked.team_id(None).unwrap(),
        Some("trusted-team".to_owned())
    );
    assert_eq!(marked.team_ids().unwrap(), vec!["trusted-plural"]);
    assert_eq!(
        marked.user_email(None).unwrap(),
        Some("trusted@example.com".to_owned())
    );
    assert_eq!(marked.org_id(None).unwrap(), Some("trusted-org".to_owned()));
    assert_eq!(
        marked.end_user_id(None).unwrap(),
        Some("trusted-end".to_owned())
    );

    let unmarked = project(RAW_CLAIMS, &settings, &configured);
    assert!(!unmarked.is_trusted_marker_token());
    assert!(
        unmarked.claims.json.get("_litellm_team_id").is_some()
            && !unmarked.is_trusted_marker_token()
    );
    assert_eq!(unmarked.user_id(None).unwrap(), Some("raw-user".to_owned()));
    assert_eq!(unmarked.team_id(None).unwrap(), Some("raw-team".to_owned()));
}

#[test]
fn marker_requires_a_configured_issuer() {
    let settings = marker_settings();
    let configured = issuers(&[ISSUER]);
    let unknown = TRUSTED_CLAIMS.replace(ISSUER, "https://other.example");
    let other = project(&unknown, &settings, &configured);
    assert_eq!(other.user_id(None).unwrap(), Some("raw-user".to_owned()));
    assert_eq!(other.team_id(None).unwrap(), Some("raw-team".to_owned()));
    assert_eq!(other.team_ids().unwrap(), vec!["raw-plural"]);
    assert_eq!(
        other.all_team_ids().unwrap(),
        vec!["raw-plural", "raw-team"]
    );

    let none_configured = project(TRUSTED_CLAIMS, &settings, &[]);
    assert_eq!(
        none_configured.user_id(None).unwrap(),
        Some("raw-user".to_owned())
    );

    let empty = TRUSTED_CLAIMS.replace(ISSUER, "");
    let empty_marker = project(&empty, &settings, &configured);
    assert!(!empty_marker.is_trusted_marker_token());
    assert_eq!(
        empty_marker.user_id(None).unwrap(),
        Some("raw-user".to_owned())
    );

    let numeric = TRUSTED_CLAIMS.replace(&format!("\"{ISSUER}\""), "5");
    let numeric_marker = project(&numeric, &settings, &configured);
    assert!(!numeric_marker.is_trusted_marker_token());
}

#[test]
fn marker_shadows_a_null_trusted_claim() {
    let settings = marker_settings();
    let configured = issuers(&[ISSUER]);
    let null_claim = r#"{
        "_litellm_jwt_issuer": "https://idp.example",
        "_litellm_user_id": null,
        "sub": "raw-user"
    }"#;
    let marked = project(null_claim, &settings, &configured);
    assert!(
        marked.is_trusted_marker_token() && marked.claims.json.get("_litellm_user_id").is_some()
    );
    assert_eq!(marked.user_id(None).unwrap(), None);
}
