use betterlite_db::database_url_policy::flag::TokenFlagError;
use betterlite_db::database_url_policy::policy::{
    ConfiguredSeconds, DatabaseTimeoutSettings, POOL_LIMIT_DEFAULT, POOL_TIMEOUT_DEFAULT,
    TimeoutError,
};
use betterlite_db::database_url_policy::query::{
    AppendQueryParam, IDLE_LIFETIME_DEFAULT, QueryValue,
};
use betterlite_db::database_urls::environment::EnvironmentSource;
use betterlite_db::database_urls::settings::{
    DatabaseUrlFlag, DatabaseUrlSettings, SettingsError, from_env,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub mod policy;
mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    database_url: Option<String>,
    #[serde(rename = "database_connection_pool_limit")]
    pool_limit: Option<i64>,
    #[serde(rename = "database_connection_timeout")]
    connection_timeout: Option<QueryValue>,
    #[serde(rename = "database_connection_pool_timeout")]
    pool_timeout: Option<QueryValue>,
    #[serde(rename = "database_connect_timeout")]
    connect_timeout: Option<QueryValue>,
    #[serde(rename = "database_socket_timeout")]
    socket_timeout: Option<QueryValue>,
    #[serde(rename = "database_disable_prepared_statements")]
    disable_prepared: Option<RawFlag>,
    #[serde(rename = "database_extra_connection_params")]
    extra_params: Option<IndexMap<String, AppendQueryParam>>,
    #[serde(rename = "database_max_idle_connection_lifetime")]
    max_idle_lifetime: Option<QueryValue>,
    #[serde(rename = "database_statement_timeout")]
    statement_timeout: Option<ConfiguredSeconds>,
    #[serde(rename = "database_lock_timeout")]
    lock_timeout: Option<ConfiguredSeconds>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<IgnoredAny>),
    Mapping(IndexMap<String, IgnoredAny>),
    Null(()),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub endpoints: DatabaseUrlSettings,
    // the raw general_settings.database_url, resolved through get_secret_str only when the CLI picks the writer URL
    pub config_url: Option<SecretString>,
    pub pool_limit: i64,
    pub pool_timeout: QueryValue,
    pub connect_timeout: Option<QueryValue>,
    pub socket_timeout: Option<QueryValue>,
    pub disable_prepared_statements: DatabaseUrlFlag,
    pub extra_params: Vec<(String, AppendQueryParam)>,
    pub idle_lifetime: QueryValue,
    pub statement_timeout: Option<f64>,
    pub lock_timeout: Option<f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoints: DatabaseUrlSettings::default(),
            config_url: None,
            pool_limit: POOL_LIMIT_DEFAULT,
            pool_timeout: QueryValue::Integer(POOL_TIMEOUT_DEFAULT),
            connect_timeout: None,
            socket_timeout: None,
            disable_prepared_statements: false,
            extra_params: Vec::new(),
            idle_lifetime: QueryValue::Integer(IDLE_LIFETIME_DEFAULT),
            statement_timeout: None,
            lock_timeout: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let environment = LoaderEnvironment { variables: lookup };
        let endpoints = from_env(&environment).map_err(settings_error)?;
        let disable_prepared = general.disable_prepared.is_some_and(flag_is_set)
            || endpoints.disable_prepared_statements;
        Ok(Settings {
            endpoints,
            config_url: general.database_url.map(SecretString::from),
            pool_limit: general.pool_limit.unwrap_or(POOL_LIMIT_DEFAULT),
            pool_timeout: general
                .connection_timeout
                .or(general.pool_timeout)
                .unwrap_or(QueryValue::Integer(POOL_TIMEOUT_DEFAULT)),
            connect_timeout: general.connect_timeout,
            socket_timeout: general.socket_timeout,
            disable_prepared_statements: disable_prepared,
            extra_params: general
                .extra_params
                .map(|entries| entries.into_iter().collect())
                .unwrap_or_default(),
            idle_lifetime: general
                .max_idle_lifetime
                .unwrap_or(QueryValue::Integer(IDLE_LIFETIME_DEFAULT)),
            statement_timeout: configured_seconds(
                general.statement_timeout,
                "general_settings.database_statement_timeout",
            )?,
            lock_timeout: configured_seconds(
                general.lock_timeout,
                "general_settings.database_lock_timeout",
            )?,
        })
    }
}

struct LoaderEnvironment<'lookup, L> {
    variables: &'lookup L,
}

impl<L> EnvironmentSource for LoaderEnvironment<'_, L>
where
    L: EnvironmentLookup,
{
    fn lookup(&self, name: &str) -> Option<String> {
        // LiteLLM's model reads names case-insensitively, so the loader is asked in the declared and folded spelling
        self.variables
            .lookup(name)
            .or_else(|| self.variables.lookup(&name.to_ascii_lowercase()))
    }
}

fn settings_error(error: SettingsError) -> ConfigError {
    match error {
        SettingsError::Flag {
            source: flag_error, ..
        } => match flag_error {
            TokenFlagError::NotRecognized { source } => ConfigError::Environment { source },
        },
        SettingsError::Lifetime { name, source } => {
            ConfigError::InvalidInteger { path: name, source }
        }
    }
}

fn configured_seconds(
    value: Option<ConfiguredSeconds>,
    path: &str,
) -> Result<Option<f64>, ConfigError> {
    let settings = DatabaseTimeoutSettings::validate(value, None)
        .map_err(|error| timeout_error(path, error))?;
    Ok(settings.database_statement_timeout)
}

fn timeout_error(path: &str, error: TimeoutError) -> ConfigError {
    match error {
        TimeoutError::NotNumeric { source, .. } => ConfigError::InvalidFloat {
            path: path.to_owned(),
            source,
        },
        // validate only parses text, so the two millisecond-conversion failures cannot occur here
        TimeoutError::NotANumber | TimeoutError::Infinite => ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        },
    }
}

// LiteLLM's str_to_bool accepts true in any case and padding; every other string is false, and nonstrings use bool()
fn flag_is_set(flag: RawFlag) -> bool {
    match flag {
        RawFlag::Boolean(value) => value,
        RawFlag::Integer(value) => value != 0,
        RawFlag::Float(value) => value != 0.0,
        RawFlag::Text(text) => matches!(text.trim().to_ascii_lowercase().as_str(), "true"),
        RawFlag::List(items) => !items.is_empty(),
        RawFlag::Mapping(entries) => !entries.is_empty(),
        RawFlag::Null(()) => false,
    }
}
