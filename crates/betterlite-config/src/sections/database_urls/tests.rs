#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_db::database_url_policy::query::{
    AppendQueryParam, QueryValue, add_missing_query_params,
};
use betterlite_db::database_urls::error::DatabaseUrlError;
use betterlite_db::database_urls::scheme::validate_cli_urls;
use betterlite_db::database_urls::settings::idle_lifetime_params;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::Settings;
use super::policy::QueryTarget;
use crate::loader::{ConfigError, ConfigLoader};

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no network in tests".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn shown(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

fn variables_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str, variables: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: variables_map(variables),
    }
    .load(source)
    .expect("config should load")
    .sections
    .database_urls
}

fn load_error(source: &str, variables: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: variables_map(variables),
    }
    .load(source)
    .expect_err("config should fail")
}

fn scalar(key: &str, value: QueryValue) -> (String, AppendQueryParam) {
    (key.to_owned(), AppendQueryParam::Scalar(value))
}

fn assert_endpoint_defaults(settings: &Settings) {
    let endpoints = &settings.endpoints;
    assert!(endpoints.database_url.is_none());
    assert!(endpoints.direct_url.is_none());
    assert_eq!(endpoints.database_host, None);
    assert_eq!(endpoints.database_port, "5432");
    assert_eq!(endpoints.database_user, None);
    assert_eq!(endpoints.database_name, None);
    assert_eq!(endpoints.database_schema, None);
    assert!(endpoints.database_password.is_none());
    assert_eq!(endpoints.database_sslmode, None);
    assert_eq!(endpoints.database_sslrootcert, None);
    assert!(endpoints.database_url_read_replica.is_none());
    let replicas = [
        &endpoints.database_host_read_replica,
        &endpoints.database_port_read_replica,
        &endpoints.database_user_read_replica,
        &endpoints.database_name_read_replica,
        &endpoints.database_schema_read_replica,
    ];
    assert!(replicas.iter().all(|value| value.is_none()));
}

fn assert_connection_defaults(settings: &Settings) {
    assert_eq!(settings.pool_limit, 10);
    assert_eq!(settings.pool_timeout, QueryValue::Integer(60));
    assert_eq!(settings.idle_lifetime, QueryValue::Integer(60));
    assert!(settings.extra_params.is_empty());
    assert_eq!(settings.endpoints.max_idle_connection_lifetime, None);
    assert!(!settings.endpoints.disable_prepared_statements);
    assert!(!settings.endpoints.iam_token_db_auth);
    assert!(!settings.endpoints.azure_postgresql_auth);
    assert!(matches!(
        (
            &settings.connect_timeout,
            &settings.socket_timeout,
            &settings.statement_timeout,
            &settings.lock_timeout,
        ),
        (None, None, None, None)
    ));
}

fn assert_alias_precedence() {
    let first_alias = load(
        "model_list: []\n",
        &[("DATABASE_USER", "first"), ("DATABASE_USERNAME", "second")],
    );
    assert_eq!(
        first_alias.endpoints.database_user.as_deref(),
        Some("first")
    );
    let empty_alias = load(
        "model_list: []\n",
        &[("DATABASE_USER", ""), ("DATABASE_USERNAME", "second")],
    );
    assert_eq!(empty_alias.endpoints.database_user.as_deref(), Some(""));
    let username_only = load("model_list: []\n", &[("DATABASE_USERNAME", "second")]);
    assert_eq!(
        username_only.endpoints.database_user.as_deref(),
        Some("second")
    );
    let reader_alias = load(
        "model_list: []\n",
        &[
            ("DATABASE_USER_READ_REPLICA", "reader"),
            ("DATABASE_USERNAME_READ_REPLICA", "ignored"),
        ],
    );
    assert_eq!(
        reader_alias.endpoints.database_user_read_replica.as_deref(),
        Some("reader")
    );
    let reader_username = load(
        "model_list: []\n",
        &[("DATABASE_USERNAME_READ_REPLICA", "reader_alias")],
    );
    assert_eq!(
        reader_username
            .endpoints
            .database_user_read_replica
            .as_deref(),
        Some("reader_alias")
    );
}

#[test]
fn env_defaults_and_alias_precedence() {
    let defaults = load("model_list: []\n", &[]);
    assert_endpoint_defaults(&defaults);
    assert_connection_defaults(&defaults);
    assert_alias_precedence();
}

#[test]
fn case_insensitive_env() {
    let lower = load(
        "model_list: []\n",
        &[
            ("database_url", "postgresql://lower/db"),
            ("database_user", "lower_user"),
            ("database_port", "6543"),
            ("database_disable_prepared_statements", "on"),
        ],
    );
    assert_eq!(
        shown(lower.endpoints.database_url.as_ref()),
        Some("postgresql://lower/db")
    );
    assert_eq!(lower.endpoints.database_user.as_deref(), Some("lower_user"));
    assert_eq!(lower.endpoints.database_port, "6543");
    assert!(lower.disable_prepared_statements);
}

#[rstest]
#[case::on("on", true)]
#[case::one("1", true)]
#[case::true_word("true", true)]
#[case::padded(" TRUE ", true)]
#[case::off("off", false)]
#[case::zero("0", false)]
#[case::empty("", false)]
fn strict_disable_env_flag(#[case] value: &str, #[case] expected: bool) {
    let settings = load(
        "model_list: []\n",
        &[("DATABASE_DISABLE_PREPARED_STATEMENTS", value)],
    );
    assert_eq!(settings.disable_prepared_statements, expected);
}

#[rstest]
#[case::integer_one("1", true)]
#[case::integer_zero("0", false)]
#[case::float_nonzero("1.5", true)]
#[case::float_zero("0.0", false)]
#[case::empty_list("[]", false)]
#[case::one_item_list("[a]", true)]
#[case::empty_mapping("{}", false)]
#[case::one_entry_mapping("{a: b}", true)]
#[case::null("null", false)]
fn nonstring_disable_flag_uses_python_truthiness(#[case] value: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  database_disable_prepared_statements: {value}\n");
    let settings = load(&source, &[]);
    assert_eq!(settings.disable_prepared_statements, expected);
}

#[test]
fn invalid_disable_env_flag_stops_loading() {
    let error = load_error(
        "model_list: []\n",
        &[("DATABASE_DISABLE_PREPARED_STATEMENTS", "sometimes")],
    );
    assert!(matches!(error, ConfigError::Environment { .. }));
}

#[test]
fn env_aliases_load_writer_fields() {
    let settings = load(
        "model_list: []\n",
        &[
            ("DATABASE_URL", "postgresql://writer/db"),
            ("DIRECT_URL", "postgresql://direct/db"),
            ("DATABASE_HOST", "db.example"),
            ("DATABASE_PORT", "6543"),
            ("DATABASE_USER", "writer"),
            ("DATABASE_NAME", "litellm"),
            ("DATABASE_SCHEMA", "tenant"),
            ("DATABASE_PASSWORD", "secret"),
            ("DATABASE_SSLMODE", "require"),
            ("DATABASE_SSLROOTCERT", "/certs/root.pem"),
        ],
    );
    let endpoints = &settings.endpoints;
    assert_eq!(
        shown(endpoints.database_url.as_ref()),
        Some("postgresql://writer/db")
    );
    assert_eq!(
        shown(endpoints.direct_url.as_ref()),
        Some("postgresql://direct/db")
    );
    assert_eq!(endpoints.database_host.as_deref(), Some("db.example"));
    assert_eq!(endpoints.database_port, "6543");
    assert_eq!(endpoints.database_user.as_deref(), Some("writer"));
    assert_eq!(endpoints.database_name.as_deref(), Some("litellm"));
    assert_eq!(endpoints.database_schema.as_deref(), Some("tenant"));
    assert!(endpoints.database_password.is_some());
    assert_eq!(endpoints.database_sslmode.as_deref(), Some("require"));
    assert_eq!(
        endpoints.database_sslrootcert.as_deref(),
        Some("/certs/root.pem")
    );
}

#[test]
fn env_aliases_load_reader_fields() {
    let settings = load(
        "model_list: []\n",
        &[
            ("DATABASE_URL_READ_REPLICA", "postgresql://reader/db"),
            ("DATABASE_HOST_READ_REPLICA", "replica.example"),
            ("DATABASE_PORT_READ_REPLICA", "5433"),
            ("DATABASE_USER_READ_REPLICA", "reader"),
            ("DATABASE_NAME_READ_REPLICA", "reader_db"),
            ("DATABASE_SCHEMA_READ_REPLICA", "reader_schema"),
            ("DATABASE_PASSWORD_READ_REPLICA", "reader_secret"),
        ],
    );
    let endpoints = &settings.endpoints;
    assert_eq!(
        shown(endpoints.database_url_read_replica.as_ref()),
        Some("postgresql://reader/db")
    );
    assert_eq!(
        endpoints.database_host_read_replica.as_deref(),
        Some("replica.example")
    );
    assert_eq!(
        endpoints.database_port_read_replica.as_deref(),
        Some("5433")
    );
    assert_eq!(
        endpoints.database_user_read_replica.as_deref(),
        Some("reader")
    );
    assert_eq!(
        endpoints.database_name_read_replica.as_deref(),
        Some("reader_db")
    );
    assert_eq!(
        endpoints.database_schema_read_replica.as_deref(),
        Some("reader_schema")
    );
    assert!(endpoints.database_password_read_replica.is_some());
}

#[test]
fn invalid_idle_integer_stops_loading() {
    let error = load_error(
        "model_list: []\n",
        &[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "not-a-number")],
    );
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. }
            if path == "DATABASE_MAX_IDLE_CONNECTION_LIFETIME"
    ));
    let loaded = load(
        "model_list: []\n",
        &[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "45")],
    );
    assert_eq!(loaded.endpoints.max_idle_connection_lifetime, Some(45));
}

#[test]
fn config_url_and_secret_precedence() {
    let no_secrets = secrets();
    let env_url = variables_map(&[("DATABASE_URL", "postgresql://env/db")]);
    let with_config_url = load(
        "general_settings:\n  database_url: postgresql://config/db\n",
        &[("DATABASE_URL", "postgresql://env/db")],
    );
    assert_eq!(
        shown(with_config_url.config_url.as_ref()),
        Some("postgresql://config/db")
    );
    assert_eq!(
        shown(
            with_config_url
                .cli_writer_url(&env_url, &no_secrets)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://config/db")
    );
    let reference = load(
        "general_settings:\n  database_url: os.environ/SET_URL\n",
        &[("SET_URL", "postgresql://reference/db")],
    );
    assert_eq!(
        shown(reference.config_url.as_ref()),
        Some("os.environ/SET_URL")
    );
    let reference_env = variables_map(&[
        ("DATABASE_URL", "postgresql://env/db"),
        ("SET_URL", "postgresql://reference/db"),
    ]);
    assert_eq!(
        shown(
            reference
                .cli_writer_url(&reference_env, &no_secrets)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://reference/db")
    );
}

#[test]
fn reference_resolving_to_bool_text_stays_absent() {
    let no_secrets = secrets();
    let settings = load(
        "general_settings:\n  database_url: os.environ/FLAG\n",
        &[("FLAG", "true")],
    );
    let environment = variables_map(&[("FLAG", "true")]);
    assert!(
        settings
            .cli_writer_url(&environment, &no_secrets)
            .unwrap()
            .is_none()
    );
}

#[test]
fn missing_reference_skips_discrete_fields() {
    let no_secrets = secrets();
    let missing_reference = load(
        "general_settings:\n  database_url: os.environ/MISSING_URL\n",
        &[],
    );
    let discrete = variables_map(&[
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USERNAME", "user"),
        ("DATABASE_NAME", "name"),
    ]);
    assert!(
        missing_reference
            .cli_writer_url(&discrete, &no_secrets)
            .unwrap()
            .is_none()
    );
}

#[test]
fn environment_url_wins_when_config_is_absent() {
    let no_secrets = secrets();
    let env_url = variables_map(&[("DATABASE_URL", "postgresql://env/db")]);
    let env_settings = load(
        "model_list: []\n",
        &[("DATABASE_URL", "postgresql://env/db")],
    );
    assert_eq!(
        shown(
            env_settings
                .cli_writer_url(&env_url, &no_secrets)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://env/db")
    );
}

#[test]
fn explicit_empty_config_url() {
    let no_secrets = secrets();
    let settings = load(
        "general_settings:\n  database_url: \"\"\n",
        &[("DATABASE_URL", "postgresql://env/db")],
    );
    assert_eq!(shown(settings.config_url.as_ref()), Some(""));
    let env_url = variables_map(&[("DATABASE_URL", "postgresql://env/db")]);
    assert_eq!(
        shown(
            settings
                .cli_writer_url(&env_url, &no_secrets)
                .unwrap()
                .as_ref()
        ),
        Some("")
    );
}

#[test]
fn cli_exact_environment_names() {
    let no_secrets = secrets();
    let settings = load("model_list: []\n", &[]);
    let lowercase_url = variables_map(&[("database_url", "mysql://lower/db")]);
    assert!(
        settings
            .cli_writer_url(&lowercase_url, &no_secrets)
            .unwrap()
            .is_none()
    );
    let mixed = variables_map(&[
        ("database_url", "mysql://lower/db"),
        ("DATABASE_HOST", "db.example"),
        ("DATABASE_USERNAME", "user"),
        ("DATABASE_NAME", "name"),
    ]);
    assert_eq!(
        shown(
            settings
                .cli_writer_url(&mixed, &no_secrets)
                .unwrap()
                .as_ref()
        ),
        Some("postgresql://user@db.example/name")
    );
    validate_cli_urls(&lowercase_url).unwrap();
}

fn assert_pool_timeout(settings: &Settings, expected: QueryValue) {
    assert_eq!(
        settings.writer_connection_params(),
        vec![
            scalar("connection_limit", QueryValue::Integer(10)),
            scalar("pool_timeout", expected)
        ]
    );
}

#[test]
fn pool_timeout_precedence() {
    let both = load(
        "general_settings:\n  database_connection_timeout: 15\n  database_connection_pool_timeout: 30\n",
        &[],
    );
    assert_pool_timeout(&both, QueryValue::Integer(15));
    let pool_only = load(
        "general_settings:\n  database_connection_pool_timeout: 30\n",
        &[],
    );
    assert_pool_timeout(&pool_only, QueryValue::Integer(30));
    let defaults = load("model_list: []\n", &[]);
    assert_pool_timeout(&defaults, QueryValue::Integer(60));
    let zero = load("general_settings:\n  database_connection_timeout: 0\n", &[]);
    assert_pool_timeout(&zero, QueryValue::Integer(0));
    let limited = load(
        "general_settings:\n  database_connection_pool_limit: 25\n",
        &[],
    );
    assert_eq!(
        limited.writer_connection_params(),
        vec![
            scalar("connection_limit", QueryValue::Integer(25)),
            scalar("pool_timeout", QueryValue::Integer(60))
        ]
    );
}

#[test]
fn optional_timeouts_and_extra_overrides() {
    let settings = load(
        "general_settings:\n  database_connect_timeout: 5\n  database_socket_timeout: 7.5\n  database_extra_connection_params:\n    pool_timeout: 99\n    application_name: litellm\n",
        &[],
    );
    assert_eq!(
        settings.writer_connection_params(),
        vec![
            scalar("connection_limit", QueryValue::Integer(10)),
            scalar("pool_timeout", QueryValue::Integer(99)),
            scalar("connect_timeout", QueryValue::Integer(5)),
            scalar("socket_timeout", QueryValue::Float(7.5)),
            scalar("application_name", QueryValue::Text("litellm".to_owned())),
        ]
    );
}

#[test]
fn config_disable_string_vs_env() {
    let string_true = load(
        "general_settings:\n  database_disable_prepared_statements: \"true\"\n",
        &[],
    );
    assert!(string_true.disable_prepared_statements);
    let string_word = load(
        "general_settings:\n  database_disable_prepared_statements: \"yes\"\n",
        &[],
    );
    assert!(!string_word.disable_prepared_statements);
    let boolean_true = load(
        "general_settings:\n  database_disable_prepared_statements: true\n",
        &[],
    );
    assert!(boolean_true.disable_prepared_statements);
    let env_flag = load(
        "general_settings:\n  database_disable_prepared_statements: false\n",
        &[("DATABASE_DISABLE_PREPARED_STATEMENTS", "on")],
    );
    assert!(env_flag.disable_prepared_statements);
    let config_wins = load(
        "general_settings:\n  database_disable_prepared_statements: \"true\"\n",
        &[("DATABASE_DISABLE_PREPARED_STATEMENTS", "off")],
    );
    assert!(config_wins.disable_prepared_statements);
}

#[test]
fn invalid_statement_timeout_stops_loading() {
    let statement = load_error(
        "general_settings:\n  database_statement_timeout: soon\n",
        &[],
    );
    assert!(matches!(
        statement,
        ConfigError::InvalidFloat { path, .. }
            if path == "general_settings.database_statement_timeout"
    ));
    let lock = load_error("general_settings:\n  database_lock_timeout: soon\n", &[]);
    assert!(matches!(
        lock,
        ConfigError::InvalidFloat { path, .. }
            if path == "general_settings.database_lock_timeout"
    ));
    let valid = load(
        "general_settings:\n  database_statement_timeout: 1.5\n  database_lock_timeout: \"2\"\n",
        &[],
    );
    assert_eq!(valid.statement_timeout, Some(1.5_f64));
    assert_eq!(valid.lock_timeout, Some(2.0_f64));
}

#[test]
fn component_idle_env_vs_cli_config() {
    let both = load(
        "general_settings:\n  database_max_idle_connection_lifetime: 45\n",
        &[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "30")],
    );
    assert_eq!(
        idle_lifetime_params(&both.endpoints),
        vec![scalar_value("max_idle_connection_lifetime", 30)]
    );
    assert_eq!(
        both.cli_idle_lifetime_params(),
        vec![scalar_value("max_idle_connection_lifetime", 45)]
    );
    let env_only = load(
        "model_list: []\n",
        &[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "30")],
    );
    assert_eq!(
        env_only.cli_idle_lifetime_params(),
        vec![scalar_value("max_idle_connection_lifetime", 60)]
    );
}

fn scalar_value(key: &str, value: i64) -> (String, QueryValue) {
    (key.to_owned(), QueryValue::Integer(value))
}

#[test]
fn pinned_idle_value_wins() {
    let settings = load(
        "model_list: []\n",
        &[("DATABASE_MAX_IDLE_CONNECTION_LIFETIME", "30")],
    );
    let url = add_missing_query_params(
        "postgresql://h/db?max_idle_connection_lifetime=90",
        &idle_lifetime_params(&settings.endpoints),
    )
    .unwrap();
    assert_eq!(url, "postgresql://h/db?max_idle_connection_lifetime=90");
}

#[test]
fn reader_query_allowlist() {
    let settings = load(
        "general_settings:\n  database_extra_connection_params:\n    schema: other\n    application_name: x\n    connection_limit: 5\n    pool_timeout: 20\n",
        &[],
    );
    assert_eq!(
        settings.reader_connection_params(),
        vec![
            scalar("connection_limit", QueryValue::Integer(5)),
            scalar("pool_timeout", QueryValue::Integer(20)),
        ]
    );
    let writer_params = settings.writer_connection_params();
    let writer_keys: Vec<&str> = writer_params.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(
        writer_keys,
        vec![
            "connection_limit",
            "pool_timeout",
            "schema",
            "application_name"
        ]
    );
}

#[test]
fn direct_no_added_statement_timeout() {
    let settings = load(
        "general_settings:\n  database_statement_timeout: 1.5\n  database_lock_timeout: 2\n",
        &[],
    );
    assert_eq!(
        settings.options_for(QueryTarget::Writer, "").unwrap(),
        "-c statement_timeout=1500 -c lock_timeout=2000"
    );
    assert_eq!(settings.options_for(QueryTarget::Direct, "").unwrap(), "");
    assert_eq!(
        settings
            .options_for(QueryTarget::Writer, "-c statement_timeout=999")
            .unwrap(),
        "-c statement_timeout=999 -c lock_timeout=2000"
    );
}

#[test]
fn present_empty_cli_url_rejected() {
    let no_secrets = secrets();
    let empty_environment = variables_map(&[("DATABASE_URL", "")]);
    let env_settings = load("model_list: []\n", &[("DATABASE_URL", "")]);
    assert_eq!(
        shown(
            env_settings
                .cli_writer_url(&empty_environment, &no_secrets)
                .unwrap()
                .as_ref()
        ),
        Some("")
    );
    let error = validate_cli_urls(&empty_environment).unwrap_err();
    assert!(matches!(error, DatabaseUrlError::UnsupportedScheme { .. }));
    assert!(
        error
            .to_string()
            .starts_with("DATABASE_URL uses unsupported scheme '<missing scheme>'")
    );
    let config_settings = load(
        "general_settings:\n  database_url: \"\"\n",
        &[("DATABASE_URL", "postgresql://h/db")],
    );
    let mut composed = variables_map(&[("DATABASE_URL", "postgresql://h/db")]);
    let selected = config_settings
        .cli_writer_url(&composed, &no_secrets)
        .unwrap()
        .expect("the explicit empty config URL wins");
    composed.insert(
        "DATABASE_URL".to_owned(),
        selected.expose_secret().to_owned(),
    );
    assert!(validate_cli_urls(&composed).is_err());
}
