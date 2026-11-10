use std::num::ParseIntError;

use secrecy::SecretString;
use serde::Deserialize;

use crate::database_url_policy::flag::{FlagValue, TokenFlagError, token_auth_flag_enabled};
use crate::database_url_policy::query::{IDLE_LIFETIME_DEFAULT, QueryValue};
use crate::database_urls::environment::EnvironmentSource;

pub const DEFAULT_POSTGRES_PORT: &str = "5432";
pub const IAM_TOKEN_DB_AUTH_ENV_VAR: &str = "IAM_TOKEN_DB_AUTH";
pub const AZURE_POSTGRESQL_AUTH_ENV_VAR: &str = "AZURE_POSTGRESQL_AUTH";
pub const DISABLE_PREPARED_ENV_VAR: &str = "DATABASE_DISABLE_PREPARED_STATEMENTS";
pub const MAX_IDLE_LIFETIME_ENV_VAR: &str = "DATABASE_MAX_IDLE_CONNECTION_LIFETIME";

pub type DatabaseUrlFlag = bool;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    Password,
    Token,
}

#[derive(Debug, Clone)]
pub struct ReaderFields {
    pub host: String,
    pub port: String,
    pub user: Option<String>,
    pub name: Option<String>,
    pub schema: Option<String>,
    pub password: Option<SecretString>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseUrlSettings {
    pub iam_token_db_auth: DatabaseUrlFlag,
    pub azure_postgresql_auth: DatabaseUrlFlag,
    pub disable_prepared_statements: DatabaseUrlFlag,
    pub max_idle_connection_lifetime: Option<i64>,
    pub database_sslmode: Option<String>,
    pub database_sslrootcert: Option<String>,
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
    pub database_host: Option<String>,
    pub database_port: String,
    pub database_user: Option<String>,
    pub database_name: Option<String>,
    pub database_schema: Option<String>,
    pub database_password: Option<SecretString>,
    pub database_url_read_replica: Option<SecretString>,
    pub database_host_read_replica: Option<String>,
    pub database_port_read_replica: Option<String>,
    pub database_user_read_replica: Option<String>,
    pub database_name_read_replica: Option<String>,
    pub database_schema_read_replica: Option<String>,
    pub database_password_read_replica: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("environment variable {name} must be a bool")]
    Flag {
        name: String,
        #[source]
        source: TokenFlagError,
    },
    #[error("environment variable {name} must be an integer")]
    Lifetime {
        name: String,
        #[source]
        source: ParseIntError,
    },
}

impl Default for DatabaseUrlSettings {
    fn default() -> Self {
        Self {
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
}

pub fn from_env<S>(environment: &S) -> Result<DatabaseUrlSettings, SettingsError>
where
    S: EnvironmentSource,
{
    Ok(DatabaseUrlSettings {
        iam_token_db_auth: flag(environment, IAM_TOKEN_DB_AUTH_ENV_VAR)?,
        azure_postgresql_auth: flag(environment, AZURE_POSTGRESQL_AUTH_ENV_VAR)?,
        disable_prepared_statements: flag(environment, DISABLE_PREPARED_ENV_VAR)?,
        max_idle_connection_lifetime: lifetime(environment)?,
        database_sslmode: environment.lookup("DATABASE_SSLMODE"),
        database_sslrootcert: environment.lookup("DATABASE_SSLROOTCERT"),
        database_url: environment.lookup("DATABASE_URL").map(SecretString::from),
        direct_url: environment.lookup("DIRECT_URL").map(SecretString::from),
        database_host: environment.lookup("DATABASE_HOST"),
        database_port: environment
            .lookup("DATABASE_PORT")
            .unwrap_or_else(|| DEFAULT_POSTGRES_PORT.to_owned()),
        database_user: environment
            .lookup("DATABASE_USER")
            .or_else(|| environment.lookup("DATABASE_USERNAME")),
        database_name: environment.lookup("DATABASE_NAME"),
        database_schema: environment.lookup("DATABASE_SCHEMA"),
        database_password: environment
            .lookup("DATABASE_PASSWORD")
            .map(SecretString::from),
        database_url_read_replica: environment
            .lookup("DATABASE_URL_READ_REPLICA")
            .map(SecretString::from),
        database_host_read_replica: environment.lookup("DATABASE_HOST_READ_REPLICA"),
        database_port_read_replica: environment.lookup("DATABASE_PORT_READ_REPLICA"),
        database_user_read_replica: environment
            .lookup("DATABASE_USER_READ_REPLICA")
            .or_else(|| environment.lookup("DATABASE_USERNAME_READ_REPLICA")),
        database_name_read_replica: environment.lookup("DATABASE_NAME_READ_REPLICA"),
        database_schema_read_replica: environment.lookup("DATABASE_SCHEMA_READ_REPLICA"),
        database_password_read_replica: environment
            .lookup("DATABASE_PASSWORD_READ_REPLICA")
            .map(SecretString::from),
    })
}

#[must_use]
pub fn idle_lifetime_params(settings: &DatabaseUrlSettings) -> Vec<(String, QueryValue)> {
    let lifetime = settings.max_idle_connection_lifetime.map_or(
        QueryValue::Integer(IDLE_LIFETIME_DEFAULT),
        QueryValue::Integer,
    );
    vec![("max_idle_connection_lifetime".to_owned(), lifetime)]
}

fn flag<S>(environment: &S, name: &str) -> Result<DatabaseUrlFlag, SettingsError>
where
    S: EnvironmentSource,
{
    let Some(text) = environment.lookup(name) else {
        return Ok(false);
    };
    let value = FlagValue::Text(text);
    token_auth_flag_enabled(Some(&value), name).map_err(|source| SettingsError::Flag {
        name: name.to_owned(),
        source,
    })
}

fn lifetime<S>(environment: &S) -> Result<Option<i64>, SettingsError>
where
    S: EnvironmentSource,
{
    let Some(text) = environment.lookup(MAX_IDLE_LIFETIME_ENV_VAR) else {
        return Ok(None);
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<i64>()
        .map(Some)
        .map_err(|source| SettingsError::Lifetime {
            name: MAX_IDLE_LIFETIME_ENV_VAR.to_owned(),
            source,
        })
}
