use rstest::rstest;

use crate::jwt_claims::claims::ClaimError;
use crate::jwt_claims::identity::IdentityClaims as _;
use crate::jwt_claims::orgs::OrgClaims as _;
use crate::jwt_claims::settings::JwtAuthSettings;

use super::support::{empty_settings, issuers, project};

fn identity_settings() -> JwtAuthSettings {
    let mut settings = empty_settings();
    settings.user_id_jwt_field = Some("sub".to_owned());
    settings.user_email_jwt_field = Some("email".to_owned());
    settings.user_allowed_email_domain = Some("example.com".to_owned());
    settings.user_id_upsert = true;
    settings.end_user_id_jwt_field = Some("eid".to_owned());
    settings.object_id_jwt_field = Some("oid".to_owned());
    settings.agent_id_jwt_field = Some("azp".to_owned());
    settings.org_id_jwt_field = Some("org".to_owned());
    settings.org_alias_jwt_field = Some("org_name".to_owned());
    settings
}

const CLAIMS: &str = r#"{
    "email": "a@example.com",
    "sub": "user-1",
    "eid": "end-1",
    "oid": "object-1",
    "azp": "agent-1",
    "org": "org-1",
    "org_name": "Org"
}"#;

#[test]
fn user_email_domain() {
    let settings = identity_settings();
    let claims = project(CLAIMS, &settings, &[]);
    assert_eq!(
        claims.user_email(None).unwrap(),
        Some("a@example.com".to_owned())
    );
    assert!(claims.is_enforced_email_domain());
    assert!(claims.is_upsert_user_id(None));
    assert!(claims.is_upsert_user_id(Some(true)));
    assert!(!claims.is_upsert_user_id(Some(false)));
}

#[test]
fn email_domain_is_enforced_even_when_empty() {
    let mut empty_domain = empty_settings();
    empty_domain.user_allowed_email_domain = Some(String::new());
    let empty = project("{}", &empty_domain, &[]);
    assert!(empty.is_enforced_email_domain());
    let default_settings = JwtAuthSettings::default();
    let no_domain = project("{}", &default_settings, &[]);
    assert!(!no_domain.is_enforced_email_domain());
    assert!(!no_domain.is_upsert_user_id(None));
}

#[test]
fn identity_claims() {
    let settings = identity_settings();
    let claims = project(CLAIMS, &settings, &[]);
    assert_eq!(claims.user_id(None).unwrap(), Some("user-1".to_owned()));
    assert_eq!(claims.end_user_id(None).unwrap(), Some("end-1".to_owned()));
    assert_eq!(claims.object_id(None).unwrap(), Some("object-1".to_owned()));
    assert_eq!(claims.agent_claim().unwrap(), Some("agent-1".to_owned()));
    assert_eq!(claims.org_id(None).unwrap(), Some("org-1".to_owned()));
    assert_eq!(claims.org_alias(None).unwrap(), Some("Org".to_owned()));
}

#[test]
fn identity_defaults_for_missing_claims() {
    let settings = identity_settings();
    let missing = project("{}", &settings, &[]);
    assert_eq!(
        missing.user_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    assert_eq!(missing.end_user_id(Some("given")).unwrap(), None);
    assert_eq!(
        missing.object_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    assert_eq!(missing.org_id(Some("given")).unwrap(), None);
    assert_eq!(missing.org_alias(Some("given")).unwrap(), None);
    assert_eq!(missing.agent_claim().unwrap(), None);
}

#[test]
fn identity_defaults_for_unset_fields() {
    let default_settings = JwtAuthSettings::default();
    let claims = project(CLAIMS, &default_settings, &[]);
    assert_eq!(
        claims.user_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    assert_eq!(claims.end_user_id(Some("given")).unwrap(), None);
    assert_eq!(
        claims.object_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    assert_eq!(claims.org_id(Some("given")).unwrap(), None);
    assert_eq!(claims.org_alias(Some("given")).unwrap(), None);
    assert_eq!(claims.agent_claim().unwrap(), None);
}

#[test]
fn user_email_type_errors() {
    let settings = identity_settings();
    let number = project(r#"{"email":5}"#, &settings, &[]);
    assert!(matches!(
        number.user_email(None),
        Err(ClaimError::Type { claim, .. }) if claim == "email"
    ));
    let null = project(r#"{"email":null}"#, &settings, &[]);
    assert_eq!(
        null.user_email(Some("fallback")).unwrap(),
        Some("fallback".to_owned())
    );
}

#[rstest]
#[case::string(r#""agent-1""#, Some("agent-1"))]
#[case::empty(r#""""#, None)]
#[case::number("5", None)]
#[case::boolean("true", None)]
#[case::null("null", None)]
fn agent_claim_accepts_only_nonempty_strings(#[case] claim: &str, #[case] expected: Option<&str>) {
    let mut settings = empty_settings();
    settings.agent_id_jwt_field = Some("azp".to_owned());
    let claims_raw = format!(r#"{{"azp":{claim}}}"#);
    let projected = project(&claims_raw, &settings, &[]);
    assert_eq!(
        projected.agent_claim().unwrap(),
        expected.map(str::to_owned)
    );
}

const TRUSTED_CLAIMS: &str = r#"{
    "_litellm_jwt_issuer": "https://idp.example",
    "_litellm_user_id": "trusted-user",
    "_litellm_user_email": "trusted@example.com",
    "_litellm_end_user_id": "trusted-end",
    "_litellm_org_id": "trusted-org",
    "sub": "raw-user",
    "email": "raw@example.com",
    "eid": "raw-end",
    "org": "raw-org",
    "oid": "raw-object"
}"#;

#[test]
fn trusted_identity_claims() {
    let settings = identity_settings();
    let configured = issuers(&["https://idp.example"]);
    let claims = project(TRUSTED_CLAIMS, &settings, &configured);
    assert_eq!(
        claims.user_id(None).unwrap(),
        Some("trusted-user".to_owned())
    );
    assert_eq!(
        claims.user_email(None).unwrap(),
        Some("trusted@example.com".to_owned())
    );
    assert_eq!(
        claims.end_user_id(None).unwrap(),
        Some("trusted-end".to_owned())
    );
    assert_eq!(claims.org_id(None).unwrap(), Some("trusted-org".to_owned()));
}

#[test]
fn object_id_has_no_trusted_marker() {
    let settings = identity_settings();
    let configured = issuers(&["https://idp.example"]);
    let claims = project(TRUSTED_CLAIMS, &settings, &configured);
    assert_eq!(
        claims.object_id(None).unwrap(),
        Some("raw-object".to_owned())
    );
}

#[test]
fn untrusted_markers_fall_back_to_raw_claims() {
    let settings = identity_settings();
    let configured = issuers(&["https://idp.example"]);
    let claims = project(
        r#"{
            "_litellm_user_id": "trusted-user",
            "_litellm_end_user_id": null,
            "sub": "raw-user",
            "eid": "raw-end"
        }"#,
        &settings,
        &configured,
    );
    assert_eq!(claims.user_id(None).unwrap(), Some("raw-user".to_owned()));
    assert_eq!(
        claims.end_user_id(None).unwrap(),
        Some("raw-end".to_owned())
    );
}
