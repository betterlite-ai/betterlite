use std::collections::BTreeMap;

use serde::de::DeserializeOwned;

use super::super::coercion::{coerce_bool, coerce_float, coerce_int};
use super::super::environment::EnvironmentLookup;
use super::super::error::RedisSettingsError;
use super::super::model::nodes::NodesInput;
use super::super::model::scalars::{BoolOrText, FloatOrText, IntOrText, PassthroughValue};

// gcp_*, azure_*, aws_iam_* and ssl_password are resolved as dedicated RedisOverrides fields (see iam::apply_iam_precedence and resolve::merge_settings) instead of through this generic REDIS_{NAME} passthrough fallback, so they are no longer listed here.
const ENV_PASSTHROUGH_KWARGS: &[&str] = &[
    "ENCODING",
    "ENCODING_ERRORS",
    "SOCKET_READ_SIZE",
    "DECODE_RESPONSES",
    "RETRY_ON_TIMEOUT",
    "RETRY_ON_ERROR",
    "CLIENT_NAME",
    "LIB_NAME",
    "LIB_VERSION",
    "PROTOCOL",
    "SOCKET_KEEPALIVE_OPTIONS",
    "SINGLE_CONNECTION_CLIENT",
    "AUTO_CLOSE_CONNECTION_POOL",
    "SSL_CERT_REQS",
    "SSL_KEYFILE",
    "SSL_CERTFILE",
    "SSL_INCLUDE_VERIFY_FLAGS",
    "SSL_EXCLUDE_VERIFY_FLAGS",
    "SSL_CHECK_HOSTNAME",
    "SSL_CA_CERTS",
    "SSL_CA_PATH",
    "SSL_CA_DATA",
    "SSL_VALIDATE_OCSP",
    "SSL_VALIDATE_OCSP_STAPLED",
    "SSL_OCSP_EXPECTED_CERT",
    "SSL_MIN_VERSION",
    "SSL_CIPHERS",
    "LEGACY_RESPONSES",
    "SOCKET_TYPE",
    "CLUSTER_ERROR_RETRY_ATTEMPTS",
    "REQUIRE_FULL_COVERAGE",
    "REINITIALIZE_STEPS",
    "READ_FROM_REPLICAS",
    "DYNAMIC_STARTUP_NODES",
    "CONNECTION_ERROR_RETRY_ATTEMPTS",
];

pub(super) fn resolve_passthrough<E>(
    mut passthrough: BTreeMap<String, PassthroughValue>,
    env: &E,
) -> BTreeMap<String, PassthroughValue>
where
    E: EnvironmentLookup,
{
    for suffix in ENV_PASSTHROUGH_KWARGS {
        let name = format!("REDIS_{suffix}");
        if let Some(value) = env.lookup(&name) {
            passthrough
                .entry(suffix.to_lowercase())
                .or_insert(PassthroughValue::Text(value));
        }
    }
    passthrough
}

pub(super) fn resolve_nodes<T, F>(
    overrides: Option<NodesInput<T>>,
    env_text: Option<String>,
    on_error: F,
) -> Result<Option<Vec<T>>, RedisSettingsError>
where
    T: DeserializeOwned,
    F: FnOnce(serde_json::Error) -> RedisSettingsError,
{
    let input = overrides.filter(NodesInput::is_truthy).or_else(|| {
        env_text
            .filter(|text| !text.is_empty())
            .map(NodesInput::Text)
    });
    match input {
        None => Ok(None),
        Some(NodesInput::Nodes(nodes)) => Ok(Some(nodes)),
        Some(NodesInput::Text(text)) => serde_json::from_str(&text).map(Some).map_err(on_error),
    }
}

// IntOrText, FloatOrText and BoolOrText each resolve the same way: an explicit override wins, otherwise the matching REDIS_{var} text falls back, and the result coerces to the scalar redis-py expects.
pub(super) trait EnvScalar: Sized {
    type Coerced;

    fn from_text(text: String) -> Self;
    fn coerce(value: Option<Self>) -> Option<Self::Coerced>;
}

impl EnvScalar for IntOrText {
    type Coerced = i64;

    fn from_text(text: String) -> Self {
        Self::Text(text)
    }

    fn coerce(value: Option<Self>) -> Option<Self::Coerced> {
        coerce_int(value)
    }
}

impl EnvScalar for FloatOrText {
    type Coerced = f64;

    fn from_text(text: String) -> Self {
        Self::Text(text)
    }

    fn coerce(value: Option<Self>) -> Option<Self::Coerced> {
        coerce_float(value)
    }
}

impl EnvScalar for BoolOrText {
    type Coerced = bool;

    fn from_text(text: String) -> Self {
        Self::Text(text)
    }

    fn coerce(value: Option<Self>) -> Option<Self::Coerced> {
        coerce_bool(value)
    }
}

pub(super) fn resolve_scalar<T, E>(
    override_value: Option<T>,
    env: &E,
    var: &str,
) -> Option<T::Coerced>
where
    T: EnvScalar,
    E: EnvironmentLookup,
{
    let raw = override_value.or_else(|| env.lookup(var).map(T::from_text));
    T::coerce(raw)
}
