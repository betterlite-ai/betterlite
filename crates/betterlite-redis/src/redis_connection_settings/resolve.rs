mod tests;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;

use super::allowlists::filter_passthrough_for_target;
use super::coercion::{coerce_bool, coerce_float, coerce_int, coerce_passthrough};
use super::environment::EnvironmentLookup;
use super::error::RedisSettingsError;
use super::model::nodes::{ClusterNodeConfig, NodesInput};
use super::model::scalars::{BoolOrText, FloatOrText, IntOrText, PassthroughValue};
use super::model::settings::{ClientKind, RedisConnectionSettings, RedisOverrides, Target};
use super::target::resolve_target;
use super::url_parts::{strip_userinfo, uses_tls};
use std::collections::BTreeMap;

const ENV_PASSTHROUGH_KWARGS: &[&str] = &[
    "ENCODING",
    "ENCODING_ERRORS",
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
    "SSL_CHECK_HOSTNAME",
    "SSL_CA_CERTS",
    "SSL_VALIDATE_OCSP",
    "SSL_VALIDATE_OCSP_STAPLED",
    "SSL_OCSP_EXPECTED_CERT",
    "CLUSTER_ERROR_RETRY_ATTEMPTS",
    "REQUIRE_FULL_COVERAGE",
    "REINITIALIZE_STEPS",
    "READ_FROM_REPLICAS",
    "DYNAMIC_STARTUP_NODES",
    "CONNECTION_ERROR_RETRY_ATTEMPTS",
    "GCP_SERVICE_ACCOUNT",
    "GCP_SSL_CA_CERTS",
    "AZURE_REDIS_AD_TOKEN",
    "AZURE_CLIENT_ID",
    "AZURE_TENANT_ID",
    "AZURE_CLIENT_SECRET",
    "AWS_IAM_AUTH",
    "AWS_IAM_USER_NAME",
    "AWS_IAM_CACHE_NAME",
    "AWS_IAM_REGION",
    "AWS_IAM_SERVERLESS",
];

// Mirrors litellm._redis._get_redis_client_logic: merge, resolve the target, coerce.
pub fn resolve_redis_kwargs<E>(
    overrides: RedisOverrides,
    client: ClientKind,
    env: &E,
) -> Result<RedisConnectionSettings, RedisSettingsError>
where
    E: EnvironmentLookup,
{
    let startup_nodes = resolve_nodes(
        overrides.startup_nodes.clone(),
        env.lookup("REDIS_CLUSTER_NODES"),
        |source| RedisSettingsError::InvalidClusterNodesJson { source },
    )?;
    let sentinel_nodes = resolve_nodes(
        overrides.sentinel_nodes.clone(),
        env.lookup("REDIS_SENTINEL_NODES"),
        |source| RedisSettingsError::InvalidSentinelNodesJson { source },
    )?;
    let mut settings = merge_settings(overrides, env, startup_nodes, sentinel_nodes);

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
    filter_passthrough_for_target(target, client, &mut settings.passthrough);
    Ok(settings)
}

fn merge_settings<E>(
    overrides: RedisOverrides,
    env: &E,
    startup_nodes: Option<Vec<ClusterNodeConfig>>,
    sentinel_nodes: Option<Vec<(String, i64)>>,
) -> RedisConnectionSettings
where
    E: EnvironmentLookup,
{
    let url = resolve_url(&overrides, env);
    RedisConnectionSettings {
        host: overrides.host.or_else(|| env.lookup("REDIS_HOST")),
        port: resolve_int(overrides.port, env, "REDIS_PORT"),
        db: resolve_int(overrides.db, env, "REDIS_DB"),
        username: overrides.username.or_else(|| env.lookup("REDIS_USERNAME")),
        password: overrides
            .password
            .or_else(|| env.lookup("REDIS_PASSWORD").map(SecretString::from)),
        url,
        ssl: resolve_bool(overrides.ssl, env, "REDIS_SSL"),
        socket_timeout: resolve_float(overrides.socket_timeout, env, "REDIS_SOCKET_TIMEOUT"),
        socket_connect_timeout: resolve_float(
            overrides.socket_connect_timeout,
            env,
            "REDIS_SOCKET_CONNECT_TIMEOUT",
        ),
        socket_keepalive: resolve_bool(overrides.socket_keepalive, env, "REDIS_SOCKET_KEEPALIVE"),
        max_connections: resolve_int(overrides.max_connections, env, "REDIS_MAX_CONNECTIONS"),
        health_check_interval: resolve_int(
            overrides.health_check_interval,
            env,
            "REDIS_HEALTH_CHECK_INTERVAL",
        ),
        credential_provider: overrides.credential_provider,
        startup_nodes,
        sentinel_nodes,
        sentinel_password: overrides.sentinel_password.or_else(|| {
            env.lookup("REDIS_SENTINEL_PASSWORD")
                .map(SecretString::from)
        }),
        service_name: overrides
            .service_name
            .or_else(|| env.lookup("REDIS_SERVICE_NAME")),
        passthrough: resolve_passthrough(overrides.passthrough, env),
    }
}

fn resolve_url<E>(overrides: &RedisOverrides, env: &E) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    let caller_named_a_target = overrides.host.is_some()
        || overrides.startup_nodes.is_some()
        || overrides.sentinel_nodes.is_some();
    overrides.url.clone().or_else(|| {
        if caller_named_a_target {
            None
        } else {
            env.lookup("REDIS_URL").map(SecretString::from)
        }
    })
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
            passthrough.insert(suffix.to_lowercase(), PassthroughValue::Text(value));
        }
    }
    passthrough
}

#[must_use]
pub fn settings_uses_tls(settings: &RedisConnectionSettings) -> bool {
    let url = settings.url.as_ref().map(ExposeSecret::expose_secret);
    uses_tls(
        settings
            .startup_nodes
            .as_ref()
            .is_some_and(|nodes| !nodes.is_empty()),
        settings.ssl,
        url,
    )
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
