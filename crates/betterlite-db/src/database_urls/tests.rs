#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::cli_url::construct_cli_database_url;
use super::error::DatabaseUrlError;
use super::password::{apply_writer_url_to_env, build_reader_url, build_writer_url};
use super::scheme::{validate_cli_urls, validate_component_schemes};
use super::settings::{
    AuthMode, DEFAULT_POSTGRES_PORT, DatabaseUrlSettings, SettingsError, from_env,
    idle_lifetime_params,
};
use super::tls::tls_params;
use crate::database_url_policy::query::QueryValue;

fn shown(url: Option<&SecretString>) -> Option<&str> {
    url.map(SecretString::expose_secret)
}

fn settings() -> DatabaseUrlSettings {
    DatabaseUrlSettings {
        iam_token_db_auth: false,
        azure_postgresql_auth: false,
        disable_prepared_statements: false,
        max_idle_connection_lifetime: None,
        database_sslmode: None,
        database_sslrootcert: None,
        database_url: None,
        direct_url: None,
        database_host: None,
        database_port: DEFAULT_POSTGRES_PORT.to_owned(),
        database_user: None,
        database_name: None,
        database_schema: None,
        database_password: None,
        database_url_read_replica: None,
        database_host_read_replica: None,
        database_port_read_replica: None,
        database_user_read_replica: None,
        database_name_read_replica: None,
        database_schema_read_replica: None,
        database_password_read_replica: None,
    }
}

fn writer() -> DatabaseUrlSettings {
    let mut settings = settings();
    settings.database_host = Some("db.example".to_owned());
    settings.database_user = Some("user".to_owned());
    settings.database_name = Some("name".to_owned());
    settings
}

fn environment_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn explicit_writer_no_replacement() {
    let mut settings = writer();
    settings.database_url = Some(SecretString::from("postgresql://pinned/db"));
    assert!(
        build_writer_url(&settings, AuthMode::Password)
            .unwrap()
            .is_none()
    );
    assert!(
        !apply_writer_url_to_env(&settings, AuthMode::Password)
            .unwrap()
            .replaced
    );
}

#[test]
fn empty_writer_assembles() {
    let mut settings = writer();
    settings.database_url = Some(SecretString::from(String::new()));
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example:5432/name")
    );
}

enum RequiredField {
    Host,
    User,
    Name,
}

#[rstest]
#[case::missing_host(RequiredField::Host)]
#[case::missing_user(RequiredField::User)]
#[case::missing_name(RequiredField::Name)]
fn required_writer_fields(#[case] missing: RequiredField) {
    let mut settings = writer();
    match missing {
        RequiredField::Host => settings.database_host = None,
        RequiredField::User => settings.database_user = None,
        RequiredField::Name => settings.database_name = None,
    }
    assert!(
        build_writer_url(&settings, AuthMode::Password)
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn password_absent_or_empty(#[case] password: Option<&str>) {
    let mut settings = writer();
    settings.database_password = password.map(SecretString::from);
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example:5432/name")
    );
}

#[test]
fn quote_plus_credentials() {
    let mut settings = writer();
    settings.database_user = Some("user name".to_owned());
    settings.database_name = Some("db/name".to_owned());
    settings.database_password = Some(SecretString::from("p@ss word"));
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user+name:p%40ss+word@db.example:5432/db%2Fname")
    );
}

#[test]
fn already_encoded_password_reencoded() {
    let mut settings = writer();
    settings.database_password = Some(SecretString::from("p%40ss"));
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user:p%2540ss@db.example:5432/name")
    );
}

#[test]
fn literal_schema_then_tls() {
    let mut settings = writer();
    settings.database_schema = Some("tenant one".to_owned());
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example:5432/name?schema=tenant one")
    );
    settings.database_sslmode = Some("require".to_owned());
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example:5432/name?schema=tenant+one&sslmode=require")
    );
    let mut empty_schema = writer();
    empty_schema.database_schema = Some(String::new());
    assert_eq!(
        shown(
            build_writer_url(&empty_schema, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example:5432/name")
    );
}

#[test]
fn root_only_verify_full() {
    let mut settings = writer();
    settings.database_sslrootcert = Some("/certs/root.pem".to_owned());
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some(
            "postgresql://user@db.example:5432/name?sslmode=verify-full&sslrootcert=%2Fcerts%2Froot.pem"
        )
    );
    settings.database_sslmode = Some(String::new());
    assert_eq!(
        tls_params(&settings),
        vec![
            (
                "sslmode".to_owned(),
                QueryValue::Text("verify-full".to_owned())
            ),
            (
                "sslrootcert".to_owned(),
                QueryValue::Text("/certs/root.pem".to_owned())
            ),
        ]
    );
}

#[test]
fn explicit_sslmode_and_blank_query() {
    let mut settings = writer();
    settings.database_sslmode = Some("require".to_owned());
    settings.database_sslrootcert = Some("/certs/root.pem".to_owned());
    settings.database_schema = Some("tenant&sslmode=".to_owned());
    assert_eq!(
        shown(
            build_writer_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some(
            "postgresql://user@db.example:5432/name?schema=tenant&sslmode=&sslrootcert=%2Fcerts%2Froot.pem"
        )
    );
}

#[test]
fn reader_host_opt_in() {
    let settings = writer();
    assert!(
        build_reader_url(&settings, AuthMode::Password)
            .unwrap()
            .is_none()
    );
    let mut empty_host = writer();
    empty_host.database_host_read_replica = Some(String::new());
    assert!(
        build_reader_url(&empty_host, AuthMode::Password)
            .unwrap()
            .is_none()
    );
}

#[test]
fn pinned_reader_no_replacement() {
    let mut settings = writer();
    settings.database_host_read_replica = Some("replica.example".to_owned());
    settings.database_url_read_replica = Some(SecretString::from("postgresql://pinned/reader"));
    assert!(
        build_reader_url(&settings, AuthMode::Password)
            .unwrap()
            .is_none()
    );
}

#[test]
fn reader_truthy_fallbacks() {
    let mut settings = writer();
    settings.database_port = "5432".to_owned();
    settings.database_schema = Some("writer_schema".to_owned());
    settings.database_password = Some(SecretString::from("writer_pass"));
    settings.database_host_read_replica = Some("replica.example".to_owned());
    settings.database_port_read_replica = Some(String::new());
    settings.database_user_read_replica = Some(String::new());
    settings.database_name_read_replica = Some(String::new());
    settings.database_schema_read_replica = Some(String::new());
    assert_eq!(
        shown(
            build_reader_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user:writer_pass@replica.example:5432/name?schema=writer_schema")
    );
    settings.database_port_read_replica = Some("6543".to_owned());
    settings.database_user_read_replica = Some("reader_user".to_owned());
    settings.database_name_read_replica = Some("reader_db".to_owned());
    settings.database_schema_read_replica = Some("reader_schema".to_owned());
    settings.database_password_read_replica = Some(SecretString::from("reader_pass"));
    assert_eq!(
        shown(
            build_reader_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some(
            "postgresql://reader_user:reader_pass@replica.example:6543/reader_db?schema=reader_schema"
        )
    );
}

#[test]
fn empty_reader_password_inherits() {
    let mut settings = writer();
    settings.database_password = Some(SecretString::from("writer_pass"));
    settings.database_host_read_replica = Some("replica.example".to_owned());
    settings.database_password_read_replica = Some(SecretString::from(""));
    assert_eq!(
        shown(
            build_reader_url(&settings, AuthMode::Password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user:writer_pass@replica.example:5432/name")
    );
}

#[test]
fn reader_does_not_parse_writer_url() {
    let mut settings = settings();
    settings.database_url = Some(SecretString::from(
        "postgresql://url_user:url_pass@pinned.example/db",
    ));
    settings.database_host_read_replica = Some("replica.example".to_owned());
    assert!(
        build_reader_url(&settings, AuthMode::Password)
            .unwrap()
            .is_none()
    );
}

#[test]
fn writer_update_result() {
    let settings = writer();
    let assembled = apply_writer_url_to_env(&settings, AuthMode::Password).unwrap();
    assert!(assembled.replaced);
    assert_eq!(assembled.updates.len(), 1);
    assert_eq!(assembled.updates[0].name, "DATABASE_URL");
    assert_eq!(
        assembled.updates[0].value.expose_secret(),
        "postgresql://user@db.example:5432/name"
    );
    let mut pinned = writer();
    pinned.database_url = Some(SecretString::from("postgresql://pinned/db"));
    let pinned_result = apply_writer_url_to_env(&pinned, AuthMode::Password).unwrap();
    assert!(!pinned_result.replaced);
    assert!(pinned_result.updates.is_empty());
}

#[test]
fn scheme_validation_order() {
    let mut writer_bad = writer();
    writer_bad.database_url = Some(SecretString::from("mysql://writer/db"));
    writer_bad.direct_url = Some(SecretString::from("sqlite://direct/db"));
    writer_bad.database_url_read_replica = Some(SecretString::from("postgres://reader/db"));
    let writer_error = validate_component_schemes(&writer_bad).unwrap_err();
    assert_eq!(
        writer_error.to_string(),
        "DATABASE_URL uses unsupported scheme 'mysql'. LiteLLM's database features (virtual keys, store_model_in_db, spend tracking) require PostgreSQL; use a 'postgresql://' connection string. SQLite and other engines are not supported. See https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
    let mut direct_bad = writer();
    direct_bad.database_url = Some(SecretString::from("postgresql://writer/db"));
    direct_bad.direct_url = Some(SecretString::from("sqlite://direct/db"));
    direct_bad.database_url_read_replica = Some(SecretString::from("postgres://reader/db"));
    let direct_error = validate_component_schemes(&direct_bad).unwrap_err();
    assert!(
        direct_error
            .to_string()
            .starts_with("DIRECT_URL uses unsupported scheme 'sqlite'")
    );
    let mut reader_bad = writer();
    reader_bad.database_url = Some(SecretString::from("postgresql://writer/db"));
    reader_bad.direct_url = Some(SecretString::from("postgres://direct/db"));
    reader_bad.database_url_read_replica = Some(SecretString::from("mysql://reader/db"));
    let reader_error = validate_component_schemes(&reader_bad).unwrap_err();
    assert!(
        reader_error
            .to_string()
            .starts_with("DATABASE_URL_READ_REPLICA uses unsupported scheme 'mysql'")
    );
    let mut all_good = writer();
    all_good.database_url = Some(SecretString::from("postgresql://writer/db"));
    all_good.direct_url = Some(SecretString::from("postgres://direct/db"));
    all_good.database_url_read_replica = Some(SecretString::from("postgresql://reader/db"));
    validate_component_schemes(&all_good).unwrap();
}

#[test]
fn falsey_component_urls_skipped() {
    let mut settings = writer();
    settings.database_url = Some(SecretString::from(String::new()));
    settings.direct_url = Some(SecretString::from(String::new()));
    settings.database_url_read_replica = Some(SecretString::from(String::new()));
    validate_component_schemes(&settings).unwrap();
}

#[test]
fn cli_url_scheme_validation() {
    let empty_url = environment_map(&[("DATABASE_URL", "")]);
    let empty_error = validate_cli_urls(&empty_url).unwrap_err();
    assert!(
        empty_error
            .to_string()
            .starts_with("DATABASE_URL uses unsupported scheme '<missing scheme>'")
    );
    let direct_bad = environment_map(&[
        ("DATABASE_URL", "postgresql://writer/db"),
        ("DIRECT_URL", "sqlite://direct/db"),
    ]);
    let direct_error = validate_cli_urls(&direct_bad).unwrap_err();
    assert!(
        direct_error
            .to_string()
            .starts_with("DIRECT_URL uses unsupported scheme 'sqlite'")
    );
    let valid = environment_map(&[
        ("DATABASE_URL", "postgresql://writer/db"),
        ("DIRECT_URL", "postgres://direct/db"),
    ]);
    validate_cli_urls(&valid).unwrap();
    validate_cli_urls(&BTreeMap::new()).unwrap();
    let lowercase = environment_map(&[("database_url", "sqlite://writer/db")]);
    validate_cli_urls(&lowercase).unwrap();
}

#[test]
fn cli_username_only_no_port() {
    let environment = environment_map(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USERNAME", "cli user"),
        ("DATABASE_USER", "ignored"),
        ("DATABASE_NAME", "cli_db"),
        ("DATABASE_PORT", "6543"),
        ("DATABASE_PASSWORD", "p@ss"),
        ("DATABASE_SCHEMA", "tenant"),
    ]);
    assert_eq!(
        shown(construct_cli_database_url(&environment).unwrap().as_ref()),
        Some("postgresql://cli+user:p%40ss@db.example/cli_db?schema=tenant")
    );
    let without_username = environment_map(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USER", "u"),
        ("DATABASE_NAME", "n"),
    ]);
    assert!(
        construct_cli_database_url(&without_username)
            .unwrap()
            .is_none()
    );
    let without_password = environment_map(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USERNAME", "user"),
        ("DATABASE_NAME", "cli_db"),
        ("DATABASE_PASSWORD", ""),
    ]);
    assert_eq!(
        shown(
            construct_cli_database_url(&without_password)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example/cli_db")
    );
}

#[test]
fn cli_lowercase_names_are_ignored() {
    let environment = environment_map(&[
        ("database_host", "db.example"),
        ("database_username", "cli user"),
        ("database_name", "cli_db"),
    ]);
    assert!(construct_cli_database_url(&environment).unwrap().is_none());
    let mixed = environment_map(&[
        ("DATABASE_HOST", "db.example"),
        ("database_username", "cli user"),
        ("DATABASE_NAME", "cli_db"),
    ]);
    assert!(construct_cli_database_url(&mixed).unwrap().is_none());
}

#[test]
fn cli_missing_fields_skips_settings_validation() {
    let missing = environment_map(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_NAME", "cli_db"),
        ("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "soon"),
    ]);
    assert!(construct_cli_database_url(&missing).unwrap().is_none());
    let complete = environment_map(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USERNAME", "user"),
        ("DATABASE_NAME", "cli_db"),
        ("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "soon"),
    ]);
    let error = construct_cli_database_url(&complete).unwrap_err();
    assert!(matches!(
        error,
        DatabaseUrlError::Settings {
            source: SettingsError::Lifetime { .. }
        }
    ));
}

#[test]
fn token_auth_yields_no_password_replacement() {
    let settings = writer();
    assert!(
        build_writer_url(&settings, AuthMode::Token)
            .unwrap()
            .is_none()
    );
    let mut reader_settings = writer();
    reader_settings.database_host_read_replica = Some("replica.example".to_owned());
    assert!(
        build_reader_url(&reader_settings, AuthMode::Token)
            .unwrap()
            .is_none()
    );
    assert!(
        !apply_writer_url_to_env(&reader_settings, AuthMode::Token)
            .unwrap()
            .replaced
    );
}

#[test]
fn from_env_reads_names_in_any_case() {
    let environment = environment_map(&[
        ("dAtAbAsE_uRl", "postgresql://mixed/db"),
        ("database_user", "lower_user"),
        ("DATABASE_USERNAME", "alias_user"),
        ("DATABASE_PORT", "6543"),
        ("DATABASE_DISABLE_PREPARED_STATEMENTS", "yes"),
        ("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", " 42 "),
    ]);
    let settings = from_env(&environment).unwrap();
    assert_eq!(
        shown(settings.database_url.as_ref()),
        Some("postgresql://mixed/db")
    );
    assert_eq!(settings.database_user.as_deref(), Some("lower_user"));
    assert_eq!(settings.database_port, "6543");
    assert!(settings.disable_prepared_statements);
    assert_eq!(settings.max_idle_connection_lifetime, Some(42));
}

#[test]
fn from_env_defaults_and_aliases() {
    let defaults = from_env(&BTreeMap::<String, String>::new()).unwrap();
    assert_eq!(defaults.database_port, DEFAULT_POSTGRES_PORT);
    assert!(defaults.database_url.is_none());
    assert!(!defaults.iam_token_db_auth);
    assert!(!defaults.azure_postgresql_auth);
    assert!(!defaults.disable_prepared_statements);
    assert_eq!(defaults.max_idle_connection_lifetime, None);
    let alias_environment = environment_map(&[
        ("DATABASE_USER", ""),
        ("DATABASE_USERNAME", "alias_user"),
        ("DATABASE_USER_READ_REPLICA", ""),
        ("DATABASE_USERNAME_READ_REPLICA", "reader_alias"),
    ]);
    let aliases = from_env(&alias_environment).unwrap();
    assert_eq!(aliases.database_user.as_deref(), Some(""));
    assert_eq!(aliases.database_user_read_replica.as_deref(), Some(""));
    let only_alias_environment = environment_map(&[("DATABASE_USERNAME", "alias_user")]);
    let only_alias = from_env(&only_alias_environment).unwrap();
    assert_eq!(only_alias.database_user.as_deref(), Some("alias_user"));
}

#[rstest]
#[case::disable_prepared("DATABASE_DISABLE_PREPARED_STATEMENTS")]
#[case::iam("IAM_TOKEN_DB_AUTH")]
#[case::azure("AZURE_POSTGRESQL_AUTH")]
fn from_env_rejects_unreadable_flags(#[case] name: &str) {
    let environment = environment_map(&[(name, "sometimes")]);
    let error = from_env(&environment).unwrap_err();
    assert!(matches!(error, SettingsError::Flag { .. }));
}

#[test]
fn from_env_rejects_unreadable_lifetimes() {
    let environment = environment_map(&[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "4.5")]);
    let error = from_env(&environment).unwrap_err();
    assert!(matches!(error, SettingsError::Lifetime { .. }));
    let empty = environment_map(&[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "")]);
    assert_eq!(from_env(&empty).unwrap().max_idle_connection_lifetime, None);
}

#[test]
fn idle_lifetime_params_use_the_env_value_or_default() {
    let settings = from_env(&BTreeMap::<String, String>::new()).unwrap();
    assert_eq!(
        idle_lifetime_params(&settings),
        vec![(
            "max_idle_connection_lifetime".to_owned(),
            QueryValue::Integer(60)
        )]
    );
    let environment = environment_map(&[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "0")]);
    let configured = from_env(&environment).unwrap();
    assert_eq!(
        idle_lifetime_params(&configured),
        vec![(
            "max_idle_connection_lifetime".to_owned(),
            QueryValue::Integer(0)
        )]
    );
}
