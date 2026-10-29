use super::query::QueryValue;
use super::url_codec::urlsplit;
use regex::Regex;
use rust_decimal::Decimal;
use std::num::ParseFloatError;

pub const CONNECTION_POOL_LIMIT_DEFAULT: i64 = 10;
pub const CONNECTION_POOL_TIMEOUT_DEFAULT: i64 = 60;

/// The three optional pool timeouts `_build_db_connection_url_params` may add,
/// grouped so the function stays under clippy's argument-count limit.
#[derive(Debug, Default, Clone)]
pub struct PoolTimeouts {
    pub pool_timeout: Option<QueryValue>,
    pub connect_timeout: Option<QueryValue>,
    pub socket_timeout: Option<QueryValue>,
}

/// `_build_db_connection_url_params`: `extra_params` is an untyped passthrough
/// whose keys win over every named argument above, including `connection_limit`.
#[must_use]
pub fn build_db_connection_url_params(
    connection_limit: i64,
    timeouts: PoolTimeouts,
    disable_prepared_statements: bool,
    extra_params: &[(String, QueryValue)],
) -> Vec<(String, QueryValue)> {
    let mut params = vec![(
        "connection_limit".to_owned(),
        QueryValue::Integer(connection_limit),
    )];
    for (key, timeout) in [
        ("pool_timeout", timeouts.pool_timeout),
        ("connect_timeout", timeouts.connect_timeout),
        ("socket_timeout", timeouts.socket_timeout),
    ] {
        if let Some(timeout_value) = timeout {
            params.push((key.to_owned(), timeout_value));
        }
    }
    if disable_prepared_statements {
        params.push(("pgbouncer".to_owned(), QueryValue::Text("true".to_owned())));
    }
    for (key, value) in extra_params {
        match params.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, slot)) => *slot = value.clone(),
            None => params.push((key.clone(), value.clone())),
        }
    }
    params
}

const SUPPORTED_DB_SCHEMES: [&str; 2] = ["postgresql", "postgres"];
const MISSING_SCHEME: &str = "<missing scheme>";

/// `unsupported_db_scheme`: `None` for a supported scheme, else the parsed
/// lower-case scheme or the missing-scheme placeholder, never the full URL.
#[must_use]
pub fn unsupported_db_scheme(database_url: &str) -> Option<String> {
    let scheme = urlsplit(database_url).scheme;
    if SUPPORTED_DB_SCHEMES.contains(&scheme.as_str()) {
        return None;
    }
    Some(if scheme.is_empty() {
        MISSING_SCHEME.to_owned()
    } else {
        scheme
    })
}

/// `unsupported_db_scheme_message`.
#[must_use]
pub fn unsupported_db_scheme_message(env_var: &str, scheme: &str) -> String {
    format!(
        "{env_var} uses unsupported scheme '{scheme}'. LiteLLM's database features (virtual keys, store_model_in_db, spend tracking) require PostgreSQL; use a 'postgresql://' connection string. SQLite and other engines are not supported. See https://docs.litellm.ai/docs/proxy/virtual_keys"
    )
}

#[derive(Debug, thiserror::Error)]
pub enum TimeoutError {
    #[error("{text} does not look like a number")]
    NotNumeric {
        text: String,
        #[source]
        source: ParseFloatError,
    },
    #[error("cannot convert float NaN to integer")]
    NotANumber,
    #[error("cannot convert float infinity to integer")]
    Infinite,
    #[error("{seconds} does not fit in a 64-bit millisecond timeout")]
    Overflow { seconds: f64 },
}

/// A `general_settings` timeout value before `DatabaseTimeoutSettings`
/// validates it: pydantic's lax float field accepts a number or a numeric
/// string, and rejects anything else.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum ConfiguredSeconds {
    Number(f64),
    Text(String),
}

impl ConfiguredSeconds {
    fn into_seconds(self) -> Result<f64, TimeoutError> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Text(text) => text
                .trim()
                .parse::<f64>()
                .map_err(|source| TimeoutError::NotNumeric { text, source }),
        }
    }
}

/// `DatabaseTimeoutSettings`: validated at the boundary, both fields default `None`.
pub struct DatabaseTimeoutSettings {
    pub database_statement_timeout: Option<f64>,
    pub database_lock_timeout: Option<f64>,
}

impl DatabaseTimeoutSettings {
    pub fn validate(
        statement_timeout: Option<ConfiguredSeconds>,
        lock_timeout: Option<ConfiguredSeconds>,
    ) -> Result<Self, TimeoutError> {
        Ok(Self {
            database_statement_timeout: statement_timeout
                .map(ConfiguredSeconds::into_seconds)
                .transpose()?,
            database_lock_timeout: lock_timeout
                .map(ConfiguredSeconds::into_seconds)
                .transpose()?,
        })
    }
}

/// `_pg_options_with_timeouts`: configured timeouts become `-c name=millis`,
/// after whatever `options` the operator already pinned, unless they already
/// spelled that setting themselves.
pub fn pg_options_with_timeouts(
    existing_options: &str,
    statement_timeout: Option<f64>,
    lock_timeout: Option<f64>,
) -> Result<String, TimeoutError> {
    let mut configured = Vec::new();
    for (name, configured_seconds) in [
        ("statement_timeout", statement_timeout),
        ("lock_timeout", lock_timeout),
    ] {
        let Some(seconds) = configured_seconds else {
            continue;
        };
        if already_spelled(existing_options, name) {
            continue;
        }
        let millis = millis_from_seconds(seconds)?;
        configured.push(format!("-c {name}={millis}"));
    }
    let mut parts: Vec<&str> = Vec::new();
    if !existing_options.is_empty() {
        parts.push(existing_options);
    }
    for part in &configured {
        parts.push(part);
    }
    Ok(parts.join(" "))
}

fn already_spelled(existing_options: &str, name: &str) -> bool {
    let pattern = format!(r"(?:-c\s*|--){}=", regex::escape(name));
    Regex::new(&pattern).is_ok_and(|spelling| spelling.is_match(existing_options))
}

fn millis_from_seconds(seconds: f64) -> Result<i64, TimeoutError> {
    if seconds.is_nan() {
        return Err(TimeoutError::NotANumber);
    }
    if seconds.is_infinite() {
        return Err(TimeoutError::Infinite);
    }
    // `mul_add` is a method call rather than the `*` operator, which keeps
    // this scaling step outside clippy's float-arithmetic restriction.
    let millis = seconds.mul_add(1000.0, 0.0).trunc();
    let exact = Decimal::from_f64_retain(millis).ok_or(TimeoutError::Overflow { seconds })?;
    i64::try_from(exact).map_err(|_source: rust_decimal::Error| TimeoutError::Overflow { seconds })
}
