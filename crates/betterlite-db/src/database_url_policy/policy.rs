use super::query::{AppendQueryParam, QueryValue};
use super::url_codec::{UrlSplitError, urlsplit};
use regex::Regex;
use std::num::ParseFloatError;

pub const POOL_LIMIT_DEFAULT: i64 = 10;
pub const POOL_TIMEOUT_DEFAULT: i64 = 60;

// Grouped so build_db_connection_url_params stays under clippy's argument-count limit.
#[derive(Debug, Default, Clone)]
pub struct PoolTimeouts {
    pub pool_timeout: Option<QueryValue>,
    pub connect_timeout: Option<QueryValue>,
    pub socket_timeout: Option<QueryValue>,
}

// extra_params is an untyped passthrough whose keys win over every named argument, including connection_limit.
#[must_use]
pub fn build_db_connection_url_params(
    connection_limit: i64,
    timeouts: PoolTimeouts,
    disable_prepared_statements: bool,
    extra_params: &[(String, AppendQueryParam)],
) -> Vec<(String, AppendQueryParam)> {
    let mut params = vec![(
        "connection_limit".to_owned(),
        AppendQueryParam::Scalar(QueryValue::Integer(connection_limit)),
    )];
    for (key, timeout) in [
        ("pool_timeout", timeouts.pool_timeout),
        ("connect_timeout", timeouts.connect_timeout),
        ("socket_timeout", timeouts.socket_timeout),
    ] {
        if let Some(timeout_value) = timeout {
            params.push((key.to_owned(), AppendQueryParam::Scalar(timeout_value)));
        }
    }
    if disable_prepared_statements {
        params.push((
            "pgbouncer".to_owned(),
            AppendQueryParam::Scalar(QueryValue::Text("true".to_owned())),
        ));
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

// Never the full URL: a caller logging this result must not get credentials back out.
pub fn unsupported_db_scheme(database_url: &str) -> Result<Option<String>, UrlSplitError> {
    let scheme = urlsplit(database_url)?.scheme;
    if SUPPORTED_DB_SCHEMES.contains(&scheme.as_str()) {
        return Ok(None);
    }
    Ok(Some(if scheme.is_empty() {
        MISSING_SCHEME.to_owned()
    } else {
        scheme
    }))
}

#[must_use]
pub fn unsupported_db_scheme_message(env_var: &str, scheme: &str) -> String {
    let mut message = format!("{env_var} uses unsupported scheme '{scheme}'. ");
    message.push_str(
        "LiteLLM's database features (virtual keys, store_model_in_db, spend tracking) require PostgreSQL; use a 'postgresql://' connection string. SQLite and other engines are not supported. See https://docs.litellm.ai/docs/proxy/virtual_keys",
    );
    message
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
}

// Pydantic's lax float field: accepts a number or a numeric string, nothing else.
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

pub struct DatabaseTimeoutSettings {
    pub database_statement_timeout: Option<f64>,
    pub database_lock_timeout: Option<f64>,
}

impl DatabaseTimeoutSettings {
    pub fn validate(
        statement_timeout: Option<ConfiguredSeconds>,
        lock_timeout: Option<ConfiguredSeconds>,
    ) -> Result<Self, TimeoutError> {
        let database_statement_timeout = statement_timeout
            .map(ConfiguredSeconds::into_seconds)
            .transpose()?;
        let database_lock_timeout = lock_timeout
            .map(ConfiguredSeconds::into_seconds)
            .transpose()?;
        Ok(Self {
            database_statement_timeout,
            database_lock_timeout,
        })
    }
}

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

// Matches Python's unbounded int(seconds * 1000): no 64-bit ceiling on the result.
fn millis_from_seconds(seconds: f64) -> Result<String, TimeoutError> {
    if seconds.is_nan() {
        return Err(TimeoutError::NotANumber);
    }
    if seconds.is_infinite() {
        return Err(TimeoutError::Infinite);
    }
    // mul_add avoids clippy's float-arithmetic-operator restriction.
    let millis = seconds.mul_add(1000.0, 0.0).trunc();
    if millis.is_infinite() {
        return Err(TimeoutError::Infinite);
    }
    Ok(format!("{:.0}", normalize_negative_zero(millis)))
}

// Python's int(seconds*1000) has no negative zero; "{:.0}" on -0.0_f64 prints "-0" unless cleared.
const fn normalize_negative_zero(value: f64) -> f64 {
    if value.to_bits() == (-0.0_f64).to_bits() {
        0.0
    } else {
        value
    }
}
