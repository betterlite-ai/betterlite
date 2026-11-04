mod lookup;
mod tests;

use std::mem;

use secrecy::{ExposeSecret, SecretString};

use self::lookup::{Coerce, resolve_nodes, resolve_passthrough};
use super::allowlists::filter_passthrough_for_target;
use super::coercion::coerce_passthrough;
use super::environment::EnvironmentLookup;
use super::error::RedisSettingsError;
use super::iam::apply_iam_precedence;
use super::model::nodes::ClusterNodeConfig;
use super::model::scalars::Scalar;
use super::model::settings::{
    ClientKind, CredentialProviderSettings, RedisConnectionSettings, RedisOverrides, Target,
};
use super::os_environ::{ReferenceNames, resolve_overrides_os_environ};
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
    let (resolved_overrides, reference_names) = resolve_overrides_os_environ(overrides, env);

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
    let mut settings = merge_settings(
        resolved_overrides.clone(),
        env,
        startup_nodes,
        sentinel_nodes,
        &reference_names,
    )?;

    apply_iam_precedence(&resolved_overrides, &mut settings, env, &reference_names)?;

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
    coerce_settings_passthrough(&mut settings, target, client, &reference_names)?;
    Ok(settings)
}

fn coerce_settings_passthrough(
    settings: &mut RedisConnectionSettings,
    target: Target,
    client: ClientKind,
    names: &ReferenceNames,
) -> Result<(), RedisSettingsError> {
    let passthrough = mem::take(&mut settings.passthrough);
    settings.passthrough = passthrough
        .into_iter()
        .filter_map(|(name, mut value)| {
            match coerce_passthrough(&name, &mut value, target, names) {
                Ok(true) => Some(Ok((name, value))),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect::<Result<_, _>>()?;
    filter_passthrough_for_target(target, client, &mut settings.passthrough)?;
    Ok(())
}

fn merge_settings<E>(
    mut overrides: RedisOverrides,
    env: &E,
    startup_nodes: Option<Vec<ClusterNodeConfig>>,
    sentinel_nodes: Option<Vec<(String, i64)>>,
    names: &ReferenceNames,
) -> Result<RedisConnectionSettings, RedisSettingsError>
where
    E: EnvironmentLookup,
{
    let mut settings = RedisConnectionSettings::default();
    resolve_network_limits(&mut settings, &mut overrides, env, names)?;
    let url = match overrides.url.clone() {
        Some(explicit) => explicit,
        None if overrides.host.is_some()
            || overrides.startup_nodes.is_some()
            || overrides.sentinel_nodes.is_some() =>
        {
            None
        }
        None => env.lookup("REDIS_URL").map(SecretString::from),
    };
    let secrets = resolve_secret_fields(&overrides, env);
    Ok(RedisConnectionSettings {
        host: overrides.host.or_else(|| env.lookup("REDIS_HOST")),
        username: overrides.username.or_else(|| env.lookup("REDIS_USERNAME")),
        password: overrides
            .password
            .unwrap_or_else(|| env.lookup("REDIS_PASSWORD").map(SecretString::from)),
        url,
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
        ..settings
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
    SecretFields {
        sentinel_password: overrides
            .sentinel_password
            .clone()
            .filter(|secret| !secret.expose_secret().is_empty())
            .or_else(|| {
                env.lookup("REDIS_SENTINEL_PASSWORD")
                    .map(SecretString::from)
            }),
        service_name: overrides
            .service_name
            .clone()
            .filter(|text| !text.is_empty())
            .or_else(|| env.lookup("REDIS_SERVICE_NAME")),
        ssl_password: overrides
            .ssl_password
            .clone()
            .or_else(|| env.lookup("REDIS_SSL_PASSWORD").map(SecretString::from)),
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

fn resolve_network_limits<E>(
    settings: &mut RedisConnectionSettings,
    overrides: &mut RedisOverrides,
    env: &E,
    names: &ReferenceNames,
) -> Result<(), RedisSettingsError>
where
    E: EnvironmentLookup,
{
    let [port, db, max_connections, health_check_interval] = [
        (overrides.port.take(), "REDIS_PORT"),
        (overrides.db.take(), "REDIS_DB"),
        (overrides.max_connections.take(), "REDIS_MAX_CONNECTIONS"),
        (
            overrides.health_check_interval.take(),
            "REDIS_HEALTH_CHECK_INTERVAL",
        ),
    ]
    .map(|(value, name)| coerce_named(name, value, env, names));
    let [ssl, socket_keepalive] = [
        (overrides.ssl.take(), "REDIS_SSL"),
        (overrides.socket_keepalive.take(), "REDIS_SOCKET_KEEPALIVE"),
    ]
    .map(|(value, name)| coerce_named(name, value, env, names));
    let [socket_timeout, socket_connect_timeout] = [
        (overrides.socket_timeout.take(), "REDIS_SOCKET_TIMEOUT"),
        (
            overrides.socket_connect_timeout.take(),
            "REDIS_SOCKET_CONNECT_TIMEOUT",
        ),
    ]
    .map(|(value, name)| coerce_named(name, value, env, names));
    settings.port = port?;
    settings.db = db?;
    settings.ssl = ssl?;
    settings.socket_timeout = socket_timeout?;
    settings.socket_connect_timeout = socket_connect_timeout?;
    settings.socket_keepalive = socket_keepalive?;
    settings.max_connections = max_connections?;
    settings.health_check_interval = health_check_interval?;
    Ok(())
}

// Coerces a REDIS_* field, blaming the referenced env var name instead of the canonical one when the override's value came from an "os.environ/<name>" reference.
fn coerce_named<T, E>(
    name: &str,
    value: Option<Scalar<T>>,
    env: &E,
    names: &ReferenceNames,
) -> Result<Option<T>, RedisSettingsError>
where
    T: Coerce,
    E: EnvironmentLookup,
{
    Coerce::coerce(
        names.get(name).map_or(name, String::as_str),
        value.or_else(|| env.lookup(name).map(Scalar::Text)),
    )
}
