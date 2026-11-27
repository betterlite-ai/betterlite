#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::{DEFAULT_USER_ID_FIELD, DEFAULT_USER_ROLE_FIELD, DEFAULT_USER_TEAM_ID_FIELD, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load(source)?.sections.oauth_tokens)
}

const LOADED_ENVIRONMENT: [(&str, &str); 6] = [
    (
        "OAUTH_TOKEN_INFO_ENDPOINT",
        "https://idp.example/introspect",
    ),
    ("OAUTH_USER_ID_FIELD_NAME", "user_id"),
    ("OAUTH_USER_ROLE_FIELD_NAME", "user_role"),
    ("OAUTH_USER_TEAM_ID_FIELD_NAME", "team_id"),
    ("OAUTH_CLIENT_ID", "client"),
    ("OAUTH_CLIENT_SECRET", "secret"),
];

const SCALAR_MAPPING: &str = "general_settings:\n  oauth2_config_mappings:\n    user_id: 1\n";
const MISSING_MAPPING: &str =
    "general_settings:\n  oauth2_config_mappings:\n    user_id: os.environ/NO_SUCH_HEADER\n";

#[test]
fn loads_settings() {
    let source = "\
general_settings:
  enable_oauth2_auth: true
  enable_oauth2_proxy_auth: true
  oauth2_config_mappings:
    user_id: X-User-Id
    models: X-Models
";
    let settings = load(source, &LOADED_ENVIRONMENT).unwrap();
    // every value below is read by betterlite-auth::oauth_tokens as this exact text
    assert_eq!(
        settings.token_info_endpoint.as_deref(),
        Some("https://idp.example/introspect")
    );
    assert_eq!(settings.user_id_field_name, "user_id");
    assert_eq!(settings.user_role_field_name, "user_role");
    assert_eq!(settings.user_team_id_field_name, "team_id");
    assert_eq!(settings.client_id.as_deref(), Some("client"));
    let secret = settings
        .client_secret
        .as_ref()
        .expect("the client secret is configured");
    assert_eq!(secret.expose_secret(), "secret");
    assert_eq!(
        settings.config_mappings,
        Some(BTreeMap::from([
            ("models".to_owned(), "X-Models".to_owned()),
            ("user_id".to_owned(), "X-User-Id".to_owned()),
        ]))
    );
    assert!(settings.enable_oauth2_auth);
    assert!(settings.enable_oauth2_proxy_auth);
}

#[test]
fn env_defaults() {
    let settings = load("{}", &[]).unwrap();
    assert_eq!(settings.token_info_endpoint, None);
    assert_eq!(settings.client_id, None);
    assert!(settings.client_secret.is_none());
    assert_eq!(settings.user_id_field_name, DEFAULT_USER_ID_FIELD);
    assert_eq!(settings.user_role_field_name, DEFAULT_USER_ROLE_FIELD);
    assert_eq!(settings.user_team_id_field_name, DEFAULT_USER_TEAM_ID_FIELD);
    assert_eq!(settings.config_mappings, None);
    assert!(!settings.enable_oauth2_auth);
    assert!(!settings.enable_oauth2_proxy_auth);
    assert_eq!(DEFAULT_USER_ID_FIELD, "sub");
    assert_eq!(DEFAULT_USER_ROLE_FIELD, "role");
    assert_eq!(DEFAULT_USER_TEAM_ID_FIELD, "team_id");
}

#[test]
fn empty_env_preserved() {
    let settings = load(
        "{}",
        &[
            ("OAUTH_TOKEN_INFO_ENDPOINT", ""),
            ("OAUTH_USER_ID_FIELD_NAME", ""),
            ("OAUTH_USER_ROLE_FIELD_NAME", ""),
            ("OAUTH_USER_TEAM_ID_FIELD_NAME", ""),
            ("OAUTH_CLIENT_ID", ""),
            ("OAUTH_CLIENT_SECRET", ""),
        ],
    )
    .unwrap();
    // os.environ.get returns "" for a set-but-empty variable: only the field-name defaults are absent-only
    assert_eq!(settings.token_info_endpoint.as_deref(), Some(""));
    assert_eq!(settings.client_id.as_deref(), Some(""));
    let secret = settings
        .client_secret
        .as_ref()
        .expect("an empty client secret is still configured");
    assert_eq!(secret.expose_secret(), "");
    assert_eq!(settings.user_id_field_name, "");
    assert_eq!(settings.user_role_field_name, "");
    assert_eq!(settings.user_team_id_field_name, "");
}

#[test]
fn mapping_direction() {
    // the dictionary keys are the UserAPIKeyAuth identity fields; the values name the request headers
    let settings = load(
        "\
general_settings:
  oauth2_config_mappings:
    user_id: X-User-Id
    user_email: X-User-Email
    team_id: X-Team-Id
    team_alias: X-Team-Alias
    org_id: X-Org-Id
    models: os.environ/MODELS_HEADER
",
        &[("MODELS_HEADER", "X-Models")],
    )
    .unwrap();
    assert_eq!(
        settings.config_mappings,
        Some(BTreeMap::from([
            ("models".to_owned(), "X-Models".to_owned()),
            ("org_id".to_owned(), "X-Org-Id".to_owned()),
            ("team_alias".to_owned(), "X-Team-Alias".to_owned()),
            ("team_id".to_owned(), "X-Team-Id".to_owned()),
            ("user_email".to_owned(), "X-User-Email".to_owned()),
            ("user_id".to_owned(), "X-User-Id".to_owned()),
        ]))
    );

    // a mapping value that is not a string names its path instead of becoming text
    let scalar = load(SCALAR_MAPPING, &[]).unwrap_err();
    assert!(matches!(
        scalar,
        ConfigError::InvalidScalar { path, .. }
            if path == "general_settings.oauth2_config_mappings.user_id"
    ));

    let missing = load(MISSING_MAPPING, &[]).unwrap_err();
    assert!(matches!(
        missing,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "NO_SUCH_HEADER"
                && path == "general_settings.oauth2_config_mappings.user_id"
    ));
}
