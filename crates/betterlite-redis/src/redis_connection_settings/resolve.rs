mod lookup;
mod tests;

use secrecy::{ExposeSecret, SecretString};

use self::lookup::{resolve_nodes, resolve_passthrough, resolve_scalar};
use super::allowlists::filter_passthrough_for_target;
use super::coercion::coerce_int;
use super::coercion::coerce_passthrough;
use super::environment::EnvironmentLookup;
use super::error::RedisSettingsError;
use super::iam::apply_iam_precedence;
use super::model::nodes::{ClusterNodeConfig, SentinelNodeInput};
use super::model::scalars::IntOrText;
use super::model::settings::{
    ClientKind, CredentialProviderSettings, RedisConnectionSettings, RedisOverrides, Target,
};
use super::os_environ::resolve_overrides_os_environ;
use super::target::resolve_target;
use super::url_parts::strip_userinfo;

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
    )?
    .map(|nodes| {
        nodes
            .into_iter()
            .map(|node| {
                let port = normalize_cluster_port(node.port)?;
                Ok(ClusterNodeConfig {
                    host: node.host,
                    port,
                })
            })
            .collect::<Result<Vec<_>, RedisSettingsError>>()
    })
    .transpose()?;
    let sentinel_nodes = resolve_nodes(
        resolved_overrides.sentinel_nodes.clone(),
        env.lookup("REDIS_SENTINEL_NODES"),
        |source| RedisSettingsError::InvalidSentinelNodesJson { source },
    )?
    .map(|nodes| {
        nodes
            .into_iter()
            .map(|node| {
                let (host, port) = match node {
                    SentinelNodeInput::Object { host, port } => (host, port),
                    SentinelNodeInput::Tuple((host, port)) => (host, port),
                };
                Ok((host, normalize_sentinel_port(port)?))
            })
            .collect::<Result<Vec<_>, RedisSettingsError>>()
    })
    .transpose()?;
    let mut settings = merge_settings(
        resolved_overrides.clone(),
        env,
        startup_nodes,
        sentinel_nodes.map(|nodes| {
            nodes
                .into_iter()
                .map(|node| (node.host, node.port))
                .collect()
        }),
    );

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

fn normalize_cluster_port(port: IntOrText) -> Result<i64, RedisSettingsError> {
    match coerce_int(Some(port)) {
        Some(port) => Ok(port),
        None => serde_json::from_str::<i64>("null")
            .map_err(|source| RedisSettingsError::InvalidClusterNodesJson { source }),
    }
}

fn normalize_sentinel_port(port: IntOrText) -> Result<i64, RedisSettingsError> {
    match coerce_int(Some(port)) {
        Some(port) => Ok(port),
        None => serde_json::from_str::<i64>("null")
            .map_err(|source| RedisSettingsError::InvalidSentinelNodesJson { source }),
    }
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
    let secrets = resolve_secret_fields(&overrides, env);
    let password = resolve_password(&overrides, env);
    RedisConnectionSettings {
        host: overrides.host.or_else(|| env.lookup("REDIS_HOST")),
        port: resolve_scalar(overrides.port, env, "REDIS_PORT"),
        db: resolve_scalar(overrides.db, env, "REDIS_DB"),
        username: overrides
            .username
            .clone()
            .or_else(|| env.lookup("REDIS_USERNAME")),
        password,
        url,
        ssl: resolve_scalar(overrides.ssl, env, "REDIS_SSL"),
        socket_timeout: resolve_scalar(overrides.socket_timeout, env, "REDIS_SOCKET_TIMEOUT"),
        socket_connect_timeout: resolve_scalar(
            overrides.socket_connect_timeout,
            env,
            "REDIS_SOCKET_CONNECT_TIMEOUT",
        ),
        socket_keepalive: resolve_scalar(overrides.socket_keepalive, env, "REDIS_SOCKET_KEEPALIVE"),
        max_connections: resolve_scalar(overrides.max_connections, env, "REDIS_MAX_CONNECTIONS"),
        health_check_interval: resolve_scalar(
            overrides.health_check_interval,
            env,
            "REDIS_HEALTH_CHECK_INTERVAL",
        ),
        credential_provider: overrides.credential_provider,
        credential_provider_settings: overrides
            .credential_provider
            .then_some(CredentialProviderSettings::Explicit),
        startup_nodes,
        sentinel_nodes,
        sentinel_password: secrets.sentinel_password,
        service_name: secrets.service_name,
        ssl_password: secrets.ssl_password,
        passthrough: resolve_passthrough(overrides.passthrough, env),
    }
}

fn resolve_password<E>(overrides: &RedisOverrides, env: &E) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    let explicit = overrides.password.clone().flatten();
    explicit.or_else(|| {
        if overrides.password.is_some() {
            None
        } else {
            env.lookup("REDIS_PASSWORD").map(SecretString::from)
        }
    })
}

struct SecretFields {
    sentinel_password: Option<SecretString>,
    service_name: Option<String>,
    ssl_password: Option<SecretString>,
}

// litellm's `redis_kwargs.get(key, None) or fallback` treats an explicit empty string the same as an absent field, so sentinel_password and service_name fall back to the environment even when the caller sent "" rather than omitting the field.
fn resolve_secret_fields<E>(overrides: &RedisOverrides, env: &E) -> SecretFields
where
    E: EnvironmentLookup,
{
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
    SecretFields {
        sentinel_password,
        service_name,
        ssl_password,
    }
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

// litellm._redis: credential_provider drops username/password and strips url userinfo.
fn apply_credential_provider(
    settings: &mut RedisConnectionSettings,
) -> Result<(), RedisSettingsError> {
    if !settings.credential_provider {
        return Ok(());
    }
    settings.username = None;
    settings.password = None;
    settings.passthrough.remove("redis_connect_func");
    if let Some(url) = settings.url.as_ref() {
        let stripped = strip_userinfo(url.expose_secret())?;
        settings.url = Some(SecretString::from(stripped));
    }
    Ok(())
}
