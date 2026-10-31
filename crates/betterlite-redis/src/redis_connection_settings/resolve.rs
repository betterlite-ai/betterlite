mod tests;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;

use super::allowlists::filter_passthrough_for_target;
use super::coercion::{coerce_bool, coerce_float, coerce_int, coerce_passthrough};
use super::environment::EnvironmentLookup;
use super::error::RedisSettingsError;
use super::iam::apply_iam_precedence;
use super::model::nodes::{ClusterNodeConfig, NodesInput};
use super::model::scalars::{BoolOrText, FloatOrText, IntOrText, PassthroughValue};
use super::model::settings::{ClientKind, RedisConnectionSettings, RedisOverrides, Target};
use super::os_environ::resolve_overrides_os_environ;
use super::target::resolve_target;
use super::url_parts::strip_userinfo;
use std::collections::BTreeMap;

// gcp_*, azure_*, aws_iam_* and ssl_password are resolved as dedicated RedisOverrides fields (see iam::apply_iam_precedence and merge_settings) instead of through this generic REDIS_{NAME} passthrough fallback, so they are no longer listed here.
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

// Mirrors litellm._redis._get_redis_client_logic: resolve os.environ/ references, merge, resolve IAM precedence, resolve the target, coerce.
pub fn resolve_redis_kwargs<E>(
    overrides: RedisOverrides,
    client: ClientKind,
    env: &E,
) -> Result<RedisConnectionSettings, RedisSettingsError>
where
    E: EnvironmentLookup,
{
    let resolved_overrides = resolve_overrides_os_environ(overrides, env);

    let startup_nodes = resolve_nodes(
        resolved_overrides.startup_nodes.clone(),
        env.lookup("REDIS_CLUSTER_NODES"),
        |source| RedisSettingsError::InvalidClusterNodesJson { source },
    )?;
    let sentinel_nodes = resolve_nodes(
        resolved_overrides.sentinel_nodes.clone(),
        env.lookup("REDIS_SENTINEL_NODES"),
        |source| RedisSettingsError::InvalidSentinelNodesJson { source },
    )?;
    let mut settings = merge_settings(&resolved_overrides, env, startup_nodes, sentinel_nodes);

    apply_iam_precedence(&resolved_overrides, &mut settings, env)?;

    let target = resolve_target(
        settings.startup_nodes.as_deref(),
        settings.sentinel_nodes.as_deref(),
        settings.url.as_ref().map(ExposeSecret::expose_secret),
        settings.host.as_deref(),
    )?;
    apply_credential_provider(&mut settings)?;
    if target == Target::Url {
        settings.host = None;
        settings.port = None;
        settings.db = None;
        settings.password = None;
    }
    coerce_passthrough(target, client, &mut settings.passthrough);
    filter_passthrough_for_target(target, client, &mut settings.passthrough)?;
    Ok(settings)
}

fn merge_settings<E>(
    overrides: &RedisOverrides,
    env: &E,
    startup_nodes: Option<Vec<ClusterNodeConfig>>,
    sentinel_nodes: Option<Vec<(String, i64)>>,
) -> RedisConnectionSettings
where
    E: EnvironmentLookup,
{
    let url = resolve_url(overrides, env);
    let (
        socket_timeout,
        socket_connect_timeout,
        socket_keepalive,
        max_connections,
        health_check_interval,
    ) = resolve_timing_fields(overrides, env);
    let (sentinel_password, service_name, ssl_password) = resolve_secret_fields(overrides, env);
    RedisConnectionSettings {
        host: overrides.host.clone().or_else(|| env.lookup("REDIS_HOST")),
        port: resolve_int(overrides.port.clone(), env, "REDIS_PORT"),
        db: resolve_int(overrides.db.clone(), env, "REDIS_DB"),
        username: overrides
            .username
            .clone()
            .or_else(|| env.lookup("REDIS_USERNAME")),
        password: overrides
            .password
            .clone()
            .or_else(|| env.lookup("REDIS_PASSWORD").map(SecretString::from)),
        url,
        ssl: resolve_bool(overrides.ssl.clone(), env, "REDIS_SSL"),
        socket_timeout,
        socket_connect_timeout,
        socket_keepalive,
        max_connections,
        health_check_interval,
        credential_provider: overrides.credential_provider,
        startup_nodes,
        sentinel_nodes,
        sentinel_password,
        service_name,
        ssl_password,
        passthrough: resolve_passthrough(overrides.passthrough.clone(), env),
    }
}

type TimingFields = (
    Option<f64>,
    Option<f64>,
    Option<bool>,
    Option<i64>,
    Option<i64>,
);

fn resolve_timing_fields<E>(overrides: &RedisOverrides, env: &E) -> TimingFields
where
    E: EnvironmentLookup,
{
    let socket_timeout = resolve_float(
        overrides.socket_timeout.clone(),
        env,
        "REDIS_SOCKET_TIMEOUT",
    );
    let socket_connect_timeout = resolve_float(
        overrides.socket_connect_timeout.clone(),
        env,
        "REDIS_SOCKET_CONNECT_TIMEOUT",
    );
    let socket_keepalive = resolve_bool(
        overrides.socket_keepalive.clone(),
        env,
        "REDIS_SOCKET_KEEPALIVE",
    );
    let max_connections = resolve_int(
        overrides.max_connections.clone(),
        env,
        "REDIS_MAX_CONNECTIONS",
    );
    let health_check_interval = resolve_int(
        overrides.health_check_interval.clone(),
        env,
        "REDIS_HEALTH_CHECK_INTERVAL",
    );
    (
        socket_timeout,
        socket_connect_timeout,
        socket_keepalive,
        max_connections,
        health_check_interval,
    )
}

type SecretFields = (Option<SecretString>, Option<String>, Option<SecretString>);

fn resolve_secret_fields<E>(overrides: &RedisOverrides, env: &E) -> SecretFields
where
    E: EnvironmentLookup,
{
    // litellm's `redis_kwargs.get(key, None) or fallback` treats an explicit empty string the same as an absent field, so sentinel_password and service_name must fall back to the environment even when the caller sent "" rather than omitting the field.
    let sentinel_password = overrides
        .sentinel_password
        .clone()
        .filter(|secret| !secret.expose_secret().is_empty())
        .or_else(|| {
            env.lookup("REDIS_SENTINEL_PASSWORD")
                .map(SecretString::from)
        });
    let service_name = overrides
        .service_name
        .clone()
        .filter(|text| !text.is_empty())
        .or_else(|| env.lookup("REDIS_SERVICE_NAME"));
    let ssl_password = overrides
        .ssl_password
        .clone()
        .or_else(|| env.lookup("REDIS_SSL_PASSWORD").map(SecretString::from));
    (sentinel_password, service_name, ssl_password)
}

fn resolve_url<E>(overrides: &RedisOverrides, env: &E) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    // litellm's `{**environment_kwargs, **env_overrides}` merge lets an explicit `url: null` overwrite an environment-sourced url instead of falling back to REDIS_URL; a present value always wins and an omitted field falls back exactly as before.
    if let Some(explicit) = overrides.url.clone() {
        return explicit;
    }
    let caller_named_a_target = overrides.host.is_some()
        || overrides.startup_nodes.is_some()
        || overrides.sentinel_nodes.is_some();
    if caller_named_a_target {
        None
    } else {
        env.lookup("REDIS_URL").map(SecretString::from)
    }
}

fn resolve_passthrough<E>(
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

fn resolve_nodes<T, F>(
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

fn resolve_int<E>(override_value: Option<IntOrText>, env: &E, var: &str) -> Option<i64>
where
    E: EnvironmentLookup,
{
    let raw = override_value.or_else(|| env.lookup(var).map(IntOrText::Text));
    coerce_int(raw)
}

fn resolve_float<E>(override_value: Option<FloatOrText>, env: &E, var: &str) -> Option<f64>
where
    E: EnvironmentLookup,
{
    let raw = override_value.or_else(|| env.lookup(var).map(FloatOrText::Text));
    coerce_float(raw)
}

fn resolve_bool<E>(override_value: Option<BoolOrText>, env: &E, var: &str) -> Option<bool>
where
    E: EnvironmentLookup,
{
    let raw = override_value.or_else(|| env.lookup(var).map(BoolOrText::Text));
    coerce_bool(raw)
}

// litellm._redis: credential_provider drops username/password and strips url userinfo.
fn apply_credential_provider(
    settings: &mut RedisConnectionSettings,
) -> Result<(), RedisSettingsError> {
    if !settings.credential_provider {
        return Ok(());
    }
    settings.username = None;
    settings.password = None;
    if let Some(url) = settings.url.as_ref() {
        let stripped = strip_userinfo(url.expose_secret())?;
        settings.url = Some(SecretString::from(stripped));
    }
    Ok(())
}
