#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_claims::claims::JwtClaims;
use betterlite_auth::jwt_claims::identity::IdentityClaims as _;
use betterlite_auth::jwt_claims::orgs::OrgClaims as _;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_claims::teams::TeamClaims as _;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::jwt_claims::Settings;

const ALL_SETTINGS: &str = r#"
general_settings:
  litellm_jwtauth:
    team_id_jwt_field: realm.team.id
    team_ids_jwt_field: realm.team.ids
    team_id_default: fallback-team
    team_alias_jwt_field: realm.team.name
    org_id_jwt_field: org
    org_alias_jwt_field: org_name
    user_id_jwt_field: sub
    user_email_jwt_field: os.environ/EMAIL_FIELD
    user_allowed_email_domain: "example.com"
    user_id_upsert: true
    end_user_id_jwt_field: eid
    agent_id_jwt_field: azp
    object_id_jwt_field: oid
"#;

const ALL_CLAIMS: &str = r#"{
    "realm": {"team": {"id": "team-1", "ids": ["team-1", "team-2"], "name": "core"}},
    "org": "org-1",
    "org_name": "Org",
    "sub": "user-1",
    "email": "a@example.com",
    "eid": "end-1",
    "azp": "agent-1",
    "oid": "object-1"
}"#;

const EMPTY_CLAIMS: &str = "{}";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load(source)?.sections.jwt_claims)
}

fn auth_settings(loaded: &Settings) -> JwtAuthSettings {
    JwtAuthSettings {
        team_id_jwt_field: loaded.team_id_jwt_field.clone(),
        team_ids_jwt_field: loaded.team_ids_jwt_field.clone(),
        team_id_default: loaded.team_id_default.clone(),
        team_alias_jwt_field: loaded.team_alias_jwt_field.clone(),
        org_id_jwt_field: loaded.org_id_jwt_field.clone(),
        org_alias_jwt_field: loaded.org_alias_jwt_field.clone(),
        user_id_jwt_field: loaded.user_id_jwt_field.clone(),
        user_email_jwt_field: loaded.user_email_jwt_field.clone(),
        user_allowed_email_domain: loaded.user_allowed_email_domain.clone(),
        user_id_upsert: loaded.user_id_upsert,
        end_user_id_jwt_field: loaded.end_user_id_jwt_field.clone(),
        agent_id_jwt_field: loaded.agent_id_jwt_field.clone(),
        object_id_jwt_field: loaded.object_id_jwt_field.clone(),
    }
}

fn project<'auth>(
    claims: &str,
    settings: &'auth JwtAuthSettings,
    issuers: &'auth [String],
) -> JwtClaims<'auth> {
    JwtClaims {
        claims: serde_json::from_str::<TypedValue>(claims).unwrap(),
        settings,
        trusted_issuers: issuers,
    }
}

#[test]
fn loads_settings() {
    let loaded = load(ALL_SETTINGS, &[("EMAIL_FIELD", "email")]).unwrap();
    assert_eq!(loaded.team_id_jwt_field.as_deref(), Some("realm.team.id"));
    assert_eq!(loaded.team_ids_jwt_field.as_deref(), Some("realm.team.ids"));
    assert_eq!(loaded.team_id_default.as_deref(), Some("fallback-team"));
    assert_eq!(
        loaded.team_alias_jwt_field.as_deref(),
        Some("realm.team.name")
    );
    assert_eq!(loaded.org_id_jwt_field.as_deref(), Some("org"));
    assert_eq!(loaded.org_alias_jwt_field.as_deref(), Some("org_name"));
    assert_eq!(loaded.user_id_jwt_field.as_deref(), Some("sub"));
    assert_eq!(loaded.user_email_jwt_field.as_deref(), Some("email"));
    assert_eq!(
        loaded.user_allowed_email_domain.as_deref(),
        Some("example.com")
    );
    assert!(loaded.user_id_upsert);
    assert_eq!(loaded.end_user_id_jwt_field.as_deref(), Some("eid"));
    assert_eq!(loaded.agent_id_jwt_field.as_deref(), Some("azp"));
    assert_eq!(loaded.object_id_jwt_field.as_deref(), Some("oid"));
    let component = auth_settings(&loaded);
    let claims = project(ALL_CLAIMS, &component, &[]);
    assert_eq!(claims.team_id(None).unwrap(), Some("team-1".to_owned()));
}

#[test]
fn loaded_settings_reach_the_projection() {
    let loaded = load(ALL_SETTINGS, &[("EMAIL_FIELD", "email")]).unwrap();
    let component = auth_settings(&loaded);
    let claims = project(ALL_CLAIMS, &component, &[]);
    assert_eq!(
        claims.team_id(Some("unused")).unwrap(),
        Some("team-1".to_owned())
    );
    assert_eq!(claims.team_ids().unwrap(), vec!["team-1", "team-2"]);
    assert_eq!(claims.team_alias(None).unwrap(), Some("core".to_owned()));
    assert_eq!(claims.org_id(None).unwrap(), Some("org-1".to_owned()));
    assert_eq!(claims.org_alias(None).unwrap(), Some("Org".to_owned()));
    assert_eq!(claims.user_id(None).unwrap(), Some("user-1".to_owned()));
    assert_eq!(
        claims.user_email(None).unwrap(),
        Some("a@example.com".to_owned())
    );
    assert!(claims.settings.user_allowed_email_domain.is_some());
    let valid_user_email: Option<bool> = None;
    assert!(valid_user_email != Some(false) && claims.settings.user_id_upsert);
    assert_eq!(claims.end_user_id(None).unwrap(), Some("end-1".to_owned()));
    assert_eq!(claims.agent_claim().unwrap(), Some("agent-1".to_owned()));
    assert_eq!(claims.object_id(None).unwrap(), Some("object-1".to_owned()));
    let missing = project(EMPTY_CLAIMS, &component, &[]);
    assert_eq!(
        missing.team_id(None).unwrap(),
        Some("fallback-team".to_owned())
    );
}

#[test]
fn defaults_and_precedence() {
    let loaded = load("", &[]).unwrap();
    assert_eq!(loaded, Settings::default());
    let component = auth_settings(&loaded);
    let claims = project(EMPTY_CLAIMS, &component, &[]);
    assert_eq!(claims.team_id(Some("given")).unwrap(), None);
    assert_eq!(claims.team_ids().unwrap(), Vec::<String>::new());
    assert_eq!(claims.team_alias(Some("given")).unwrap(), None);
    assert_eq!(claims.org_id(Some("given")).unwrap(), None);
    assert_eq!(claims.org_alias(Some("given")).unwrap(), None);
    assert_eq!(
        claims.user_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    assert_eq!(claims.user_email(Some("given")).unwrap(), None);
    assert_eq!(claims.end_user_id(Some("given")).unwrap(), None);
    assert_eq!(
        claims.object_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    assert_eq!(claims.agent_claim().unwrap(), None);
    assert!(claims.settings.user_allowed_email_domain.is_none());
    assert!(!claims.settings.user_id_upsert);
    assert!(
        claims.settings.team_id_jwt_field.is_none()
            && claims.settings.team_alias_jwt_field.is_none()
    );
}

#[test]
fn null_keys_take_defaults() {
    let nulls = load(
        "general_settings:\n  litellm_jwtauth:\n    team_id_jwt_field: null\n    user_id_upsert: null\n",
        &[],
    )
    .unwrap();
    assert_eq!(nulls, Settings::default());
}

#[test]
fn explicit_values_and_environment() {
    let explicit = load(
        "general_settings:\n  litellm_jwtauth:\n    team_id_jwt_field: \"\"\n    user_allowed_email_domain: \"\"\n",
        &[],
    )
    .unwrap();
    assert_eq!(explicit.team_id_jwt_field.as_deref(), Some(""));
    let component = auth_settings(&explicit);
    let claims = project(EMPTY_CLAIMS, &component, &[]);
    assert!(claims.settings.user_allowed_email_domain.is_some());
    assert!(
        claims.settings.team_id_jwt_field.is_some()
            || claims.settings.team_alias_jwt_field.is_some()
    );

    let environment = load(
        "general_settings:\n  litellm_jwtauth:\n    team_id_jwt_field: os.environ/TEAM_FIELD\n",
        &[("TEAM_FIELD", "resolved.team")],
    )
    .unwrap();
    assert_eq!(
        environment.team_id_jwt_field.as_deref(),
        Some("resolved.team")
    );
    let unset = load(
        "general_settings:\n  litellm_jwtauth:\n    team_id_jwt_field: os.environ/TEAM_FIELD\n",
        &[("TEAM_FIELD", "")],
    )
    .unwrap();
    assert_eq!(unset.team_id_jwt_field, None);

    let upsert = load(
        "general_settings:\n  litellm_jwtauth:\n    user_id_upsert: os.environ/UPSERT\n",
        &[("UPSERT", "yes")],
    )
    .unwrap();
    assert!(upsert.user_id_upsert);
    let upsert_unset = load(
        "general_settings:\n  litellm_jwtauth:\n    user_id_upsert: os.environ/UPSERT\n",
        &[("UPSERT", "")],
    )
    .unwrap();
    assert!(!upsert_unset.user_id_upsert);
}

#[rstest]
#[case::number_for_text(
    "general_settings:\n  litellm_jwtauth:\n    team_id_jwt_field: 5\n",
    &[],
    "general_settings.litellm_jwtauth.team_id_jwt_field",
    true
)]
#[case::list_for_text(
    "general_settings:\n  litellm_jwtauth:\n    user_email_jwt_field: [email]\n",
    &[],
    "general_settings.litellm_jwtauth.user_email_jwt_field",
    true
)]
#[case::text_for_flag(
    "general_settings:\n  litellm_jwtauth:\n    user_id_upsert: \"yes\"\n",
    &[],
    "general_settings.litellm_jwtauth.user_id_upsert",
    true
)]
#[case::number_for_flag(
    "general_settings:\n  litellm_jwtauth:\n    user_id_upsert: 1\n",
    &[],
    "general_settings.litellm_jwtauth.user_id_upsert",
    true
)]
#[case::missing_reference(
    "general_settings:\n  litellm_jwtauth:\n    team_id_jwt_field: os.environ/TEAM_FIELD\n",
    &[],
    "TEAM_FIELD",
    false
)]
#[case::invalid_flag_reference(
    "general_settings:\n  litellm_jwtauth:\n    user_id_upsert: os.environ/UPSERT\n",
    &[("UPSERT", "maybe")],
    "UPSERT",
    false
)]
fn invalid_value_fallback(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] fragment: &str,
    #[case] scalar: bool,
) {
    let error = load(source, environment).unwrap_err();
    assert!(error.to_string().contains(fragment), "{error}");
    assert_eq!(matches!(&error, ConfigError::InvalidScalar { .. }), scalar);
}
