#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as _;
use std::fmt;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::settings::settings_resolution::FieldSource;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::{
    ResolvedSsoConfig, SSO_DESCRIPTORS, SsoResolutionError, decrypt_row, resolve_sso_config,
    sso_field_env_vars, sso_secret_fields,
};
use crate::sso_types::records::{TeamMappings, UiAccessMode};
use crate::user_roles::UserRole;

const SALT: &str = "salt-one";

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn sealed(cipher: &DbCipher, plaintext: &str) -> JsonValue {
    let encrypted = cipher
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(plaintext)), None)
        .unwrap();
    let StoredValue::Text(text) = encrypted else {
        panic!("a string encrypts to a string");
    };
    JsonValue::String(text.expose_secret().to_owned())
}

fn row(entries: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

fn config_of(
    database: &BTreeMap<String, JsonValue>,
    env: &BTreeMap<String, SecretString>,
) -> ResolvedSsoConfig {
    resolve_sso_config(Some(database), env, &cipher()).unwrap()
}

#[test]
fn secrets_come_back_unmasked() {
    let cipher = cipher();
    let database = row(&[
        ("microsoft_client_id", sealed(&cipher, "ms-client")),
        ("microsoft_client_secret", sealed(&cipher, "ms-secret")),
        ("microsoft_tenant", JsonValue::from("tenant-1")),
    ]);
    let config = resolve_sso_config(Some(&database), &BTreeMap::new(), &cipher)
        .unwrap()
        .config;

    assert_eq!(config.microsoft_client_id.as_deref(), Some("ms-client"));
    assert_eq!(
        config
            .microsoft_client_secret
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Some("ms-secret".to_owned())
    );
    assert_eq!(config.microsoft_tenant.as_deref(), Some("tenant-1"));
}

#[test]
fn provenance_names_every_descriptor_field_and_its_source() {
    let cipher = cipher();
    let database = row(&[
        ("microsoft_client_id", sealed(&cipher, "ms-client")),
        ("microsoft_client_secret", JsonValue::from("ms-secret")),
    ]);
    let env = environment(&[("GOOGLE_CLIENT_ID", "google-id")]);
    let resolved = resolve_sso_config(Some(&database), &env, &cipher).unwrap();

    assert_eq!(resolved.provenance.len(), 16);
    assert_eq!(
        resolved.provenance.get("microsoft_client_id"),
        Some(&FieldSource::Db)
    );
    assert_eq!(
        resolved.provenance.get("microsoft_client_secret"),
        Some(&FieldSource::Db)
    );
    assert_eq!(
        resolved.provenance.get("google_client_id"),
        Some(&FieldSource::Env)
    );
    assert_eq!(
        resolved.provenance.get("google_client_secret"),
        Some(&FieldSource::Unset)
    );
    assert_eq!(
        resolved.provenance.get("generic_scope"),
        Some(&FieldSource::Default)
    );
}

#[test]
fn a_missing_database_row_resolves_from_the_environment() {
    let env = environment(&[("GOOGLE_CLIENT_ID", "env-id")]);
    let resolved = resolve_sso_config(None, &env, &cipher()).unwrap();

    assert_eq!(resolved.config.google_client_id.as_deref(), Some("env-id"));
    assert_eq!(
        resolved.config.generic_scope.as_deref(),
        Some("openid email profile")
    );
    assert_eq!(resolved.provenance.len(), 16);
}

#[test]
fn decrypt_row_decrypts_only_strings_and_skips_structured_keys() {
    let cipher = cipher();
    let raw = row(&[
        ("google_client_id", sealed(&cipher, "plain-id")),
        ("saml_allow_unsolicited", JsonValue::Bool(true)),
        ("generic_scope", JsonValue::from(5)),
        ("saml_idp_metadata_xml", JsonValue::Null),
        ("proxy_base_url", JsonValue::from("not-a-ciphertext")),
        ("role_mappings", JsonValue::from("sealed")),
        ("team_mappings", JsonValue::Null),
    ]);
    let decrypted = decrypt_row(&raw, &cipher);

    assert_eq!(
        decrypted.get("google_client_id"),
        Some(&JsonValue::from("plain-id"))
    );
    assert_eq!(
        decrypted.get("saml_allow_unsolicited"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(decrypted.get("generic_scope"), Some(&JsonValue::from(5)));
    assert_eq!(
        decrypted.get("saml_idp_metadata_xml"),
        Some(&JsonValue::Null)
    );
    assert_eq!(
        decrypted.get("proxy_base_url"),
        Some(&JsonValue::from("not-a-ciphertext"))
    );
    assert!(!decrypted.contains_key("role_mappings"));
    assert!(!decrypted.contains_key("team_mappings"));
    assert_eq!(decrypted.len(), 5);
}

#[derive(Clone, Default)]
struct RecordedEvents(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for RecordedEvents {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut messages = Messages::default();
        event.record(&mut messages);
        self.0.lock().unwrap().extend(messages.0);
    }
}

#[derive(Default)]
struct Messages(Vec<String>);

impl Visit for Messages {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }
}

#[test]
fn an_undecryptable_value_logs_the_failure_and_stays_as_stored() {
    let database = row(&[("google_client_id", JsonValue::from("not-a-ciphertext"))]);
    let events = RecordedEvents::default();
    let subscriber = tracing_subscriber::registry().with(events.clone());
    let config = with_default(subscriber, || {
        resolve_sso_config(Some(&database), &BTreeMap::new(), &cipher())
            .unwrap()
            .config
    });

    assert_eq!(config.google_client_id.as_deref(), Some("not-a-ciphertext"));
    let recorded = events.0.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].contains("Unable to decrypt value for key: google_client_id"));
}

#[rstest]
#[case::database_wins(r#""db-id""#, Some("db-id"), FieldSource::Db)]
#[case::null_falls_back("null", Some("env-id"), FieldSource::Env)]
#[case::blank_falls_back(r#""""#, Some("env-id"), FieldSource::Env)]
#[case::not_a_string_falls_back("7", Some("env-id"), FieldSource::Env)]
fn database_values_shadow_the_environment_only_when_set(
    #[case] stored: &str,
    #[case] expected: Option<&str>,
    #[case] source: FieldSource,
) {
    let database = row(&[("google_client_id", serde_json::from_str(stored).unwrap())]);
    let env = environment(&[("GOOGLE_CLIENT_ID", "env-id")]);
    let resolved = resolve_sso_config(Some(&database), &env, &cipher()).unwrap();

    assert_eq!(resolved.config.google_client_id.as_deref(), expected);
    assert_eq!(resolved.provenance.get("google_client_id"), Some(&source));
}

#[rstest]
#[case::default_scope(None, &[], Some("openid email profile"), FieldSource::Default)]
#[case::database_wins(
    Some("db-scope"),
    &[("GENERIC_SCOPE", "env-scope")],
    Some("db-scope"),
    FieldSource::Db
)]
#[case::environment_without_database(
    None,
    &[("GENERIC_SCOPE", "env-scope")],
    Some("env-scope"),
    FieldSource::Env
)]
#[case::blank_database_falls_through(
    Some(""),
    &[("GENERIC_SCOPE", "env-scope")],
    Some("env-scope"),
    FieldSource::Env
)]
#[case::blank_environment_keeps_the_default(
    None,
    &[("GENERIC_SCOPE", "  ")],
    Some("openid email profile"),
    FieldSource::Default
)]
fn generic_scope_defaults_and_overrides(
    #[case] database_scope: Option<&str>,
    #[case] env_entries: &[(&str, &str)],
    #[case] expected: Option<&str>,
    #[case] source: FieldSource,
) {
    let database = database_scope.map_or_else(BTreeMap::new, |scope| {
        row(&[("generic_scope", JsonValue::from(scope))])
    });
    let env = environment(env_entries);
    let resolved = resolve_sso_config(Some(&database), &env, &cipher()).unwrap();

    assert_eq!(resolved.config.generic_scope.as_deref(), expected);
    assert_eq!(resolved.provenance.get("generic_scope"), Some(&source));
}

#[rstest]
#[case::text(r#""text""#)]
#[case::number("7")]
#[case::boolean("false")]
#[case::null("null")]
#[case::array("[1, 2]")]
fn role_mappings_parse_only_from_a_json_object(#[case] stored: &str) {
    let database = row(&[("role_mappings", serde_json::from_str(stored).unwrap())]);
    let config = config_of(&database, &BTreeMap::new()).config;

    assert_eq!(config.role_mappings, None);
}

#[rstest]
#[case::text(r#""text""#)]
#[case::number("7")]
#[case::boolean("false")]
#[case::null("null")]
#[case::array("[1, 2]")]
fn team_mappings_parse_only_from_a_json_object(#[case] stored: &str) {
    let database = row(&[("team_mappings", serde_json::from_str(stored).unwrap())]);
    let config = config_of(&database, &BTreeMap::new()).config;

    assert_eq!(config.team_mappings, None);
}

#[test]
fn role_mappings_parse_their_provider_claim_and_roles() {
    let database = row(&[(
        "role_mappings",
        serde_json::from_str(
            r#"{"provider":"google","group_claim":"groups","default_role":"internal_user","roles":{"proxy_admin":["admins"]}}"#,
        )
        .unwrap(),
    )]);
    let config = config_of(&database, &BTreeMap::new()).config;
    let mappings = config.role_mappings.unwrap();

    assert_eq!(mappings.provider, "google");
    assert_eq!(mappings.group_claim, "groups");
    assert_eq!(mappings.default_role, Some(UserRole::InternalUser));
    assert_eq!(
        mappings.roles,
        vec![(UserRole::ProxyAdmin, vec!["admins".to_owned()])]
    );
}

#[test]
fn team_mappings_parse_their_jwt_field() {
    let database = row(&[(
        "team_mappings",
        serde_json::from_str(r#"{"team_ids_jwt_field":"teams"}"#).unwrap(),
    )]);
    let config = config_of(&database, &BTreeMap::new()).config;

    assert_eq!(
        config.team_mappings,
        Some(TeamMappings {
            team_ids_jwt_field: Some("teams".to_owned()),
        })
    );
}

#[test]
fn an_encrypted_mapping_string_is_not_decrypted_into_a_mapping() {
    let cipher = cipher();
    let database = row(&[
        (
            "role_mappings",
            sealed(&cipher, r#"{"provider":"google","group_claim":"groups"}"#),
        ),
        (
            "team_mappings",
            sealed(&cipher, r#"{"team_ids_jwt_field":"teams"}"#),
        ),
    ]);
    let config = resolve_sso_config(Some(&database), &BTreeMap::new(), &cipher)
        .unwrap()
        .config;

    assert_eq!(config.role_mappings, None);
    assert_eq!(config.team_mappings, None);
}

#[test]
fn user_email_and_ui_access_mode_come_from_the_database_only() {
    let cipher = cipher();
    let database = row(&[
        ("user_email", sealed(&cipher, "admin@example.com")),
        ("ui_access_mode", JsonValue::from("admin_only")),
    ]);
    let env = environment(&[
        ("USER_EMAIL", "env@example.com"),
        ("UI_ACCESS_MODE", "env-mode"),
    ]);
    let resolved = resolve_sso_config(Some(&database), &env, &cipher).unwrap();

    assert_eq!(
        resolved.config.user_email.as_deref(),
        Some("admin@example.com")
    );
    assert_eq!(
        resolved.config.ui_access_mode,
        Some(UiAccessMode::Value("admin_only".to_owned()))
    );
    assert!(!resolved.provenance.contains_key("user_email"));
    assert!(!resolved.provenance.contains_key("ui_access_mode"));

    let empty = resolve_sso_config(Some(&BTreeMap::new()), &env, &cipher).unwrap();
    assert_eq!(empty.config.user_email, None);
    assert_eq!(empty.config.ui_access_mode, None);
}

#[test]
fn null_structured_values_are_absent() {
    let database = row(&[
        ("user_email", JsonValue::Null),
        ("ui_access_mode", JsonValue::Null),
        ("role_mappings", JsonValue::Null),
        ("team_mappings", JsonValue::Null),
    ]);
    let config = config_of(&database, &BTreeMap::new()).config;

    assert_eq!(config.user_email, None);
    assert_eq!(config.ui_access_mode, None);
    assert_eq!(config.role_mappings, None);
    assert_eq!(config.team_mappings, None);
}

#[test]
fn ui_access_mode_parses_a_restricted_object() {
    let database = row(&[(
        "ui_access_mode",
        serde_json::from_str(
            r#"{"type":"restricted_sso_group","restricted_sso_group":"admins","sso_group_jwt_field":"groups"}"#,
        )
        .unwrap(),
    )]);
    let config = config_of(&database, &BTreeMap::new()).config;

    assert!(matches!(
        config.ui_access_mode,
        Some(UiAccessMode::Restricted(_))
    ));
}

#[rstest]
#[case::user_email_number("user_email", "5")]
#[case::user_email_array("user_email", "[1]")]
#[case::ui_access_mode_number("ui_access_mode", "5")]
#[case::ui_access_mode_unknown_type(
    "ui_access_mode",
    r#"{"type":"other","restricted_sso_group":"admins","sso_group_jwt_field":"groups"}"#
)]
#[case::role_mappings_missing_provider("role_mappings", r#"{"group_claim":"groups"}"#)]
#[case::role_mappings_unknown_role(
    "role_mappings",
    r#"{"provider":"google","group_claim":"groups","roles":{"superuser":["admins"]}}"#
)]
#[case::team_mappings_wrong_field_type("team_mappings", r#"{"team_ids_jwt_field":5}"#)]
fn invalid_structured_values_name_their_field(#[case] field: &'static str, #[case] stored: &str) {
    let database = row(&[(field, serde_json::from_str(stored).unwrap())]);
    let error = resolve_sso_config(Some(&database), &BTreeMap::new(), &cipher()).unwrap_err();

    let SsoResolutionError::Field {
        field: named,
        source,
    } = &error;
    assert_eq!(*named, field);
    assert!(source.source().is_some());
    assert!(error.to_string().contains(field));
}

const EXPECTED_FIELDS: [(&str, &str, bool, Option<&str>); 16] = [
    ("google_client_id", "GOOGLE_CLIENT_ID", false, None),
    ("google_client_secret", "GOOGLE_CLIENT_SECRET", true, None),
    ("microsoft_client_id", "MICROSOFT_CLIENT_ID", false, None),
    (
        "microsoft_client_secret",
        "MICROSOFT_CLIENT_SECRET",
        true,
        None,
    ),
    ("microsoft_tenant", "MICROSOFT_TENANT", false, None),
    ("generic_client_id", "GENERIC_CLIENT_ID", false, None),
    ("generic_client_secret", "GENERIC_CLIENT_SECRET", true, None),
    (
        "generic_authorization_endpoint",
        "GENERIC_AUTHORIZATION_ENDPOINT",
        false,
        None,
    ),
    (
        "generic_token_endpoint",
        "GENERIC_TOKEN_ENDPOINT",
        false,
        None,
    ),
    (
        "generic_userinfo_endpoint",
        "GENERIC_USERINFO_ENDPOINT",
        false,
        None,
    ),
    (
        "generic_scope",
        "GENERIC_SCOPE",
        false,
        Some("openid email profile"),
    ),
    (
        "saml_idp_metadata_url",
        "SAML_IDP_METADATA_URL",
        false,
        None,
    ),
    (
        "saml_idp_metadata_xml",
        "SAML_IDP_METADATA_XML",
        false,
        None,
    ),
    ("saml_sp_entity_id", "SAML_SP_ENTITY_ID", false, None),
    (
        "saml_allow_unsolicited",
        "SAML_ALLOW_UNSOLICITED",
        false,
        None,
    ),
    ("proxy_base_url", "PROXY_BASE_URL", false, None),
];

#[test]
fn the_descriptor_table_matches_the_litellm_field_order() {
    let fields = SSO_DESCRIPTORS
        .iter()
        .map(|descriptor| {
            (
                descriptor.field_name,
                descriptor.env_var,
                descriptor.is_secret,
                descriptor.default,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(fields, EXPECTED_FIELDS);
    assert!(
        SSO_DESCRIPTORS
            .iter()
            .all(|descriptor| descriptor.db_key == descriptor.field_name)
    );
}

#[test]
fn the_derived_tables_match_the_descriptor_table() {
    let secrets = SSO_DESCRIPTORS
        .iter()
        .filter(|descriptor| descriptor.is_secret)
        .map(|descriptor| descriptor.field_name)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        secrets,
        BTreeSet::from([
            "generic_client_secret",
            "google_client_secret",
            "microsoft_client_secret",
        ])
    );
    assert_eq!(sso_secret_fields(), secrets);
    let env_vars = SSO_DESCRIPTORS
        .iter()
        .map(|descriptor| (descriptor.field_name, descriptor.env_var))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(sso_field_env_vars(), env_vars);
    assert!(!sso_field_env_vars().contains_key("user_email"));
    assert!(!sso_field_env_vars().contains_key("ui_access_mode"));
}
