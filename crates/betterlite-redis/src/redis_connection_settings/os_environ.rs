mod tests;

use secrecy::{ExposeSecret as _, SecretString};

use super::environment::EnvironmentLookup;
use super::model::nodes::NodesInput;
use super::model::scalars::{BoolOrText, FloatOrText, IntOrText, PassthroughValue};
use super::model::settings::RedisOverrides;

const PREFIX: &str = "os.environ/";

// litellm._redis._get_redis_client_logic resolves every "os.environ/<name>" string in env_overrides through get_secret before merging with the environment, parsing startup or sentinel nodes, or coercing numeric and boolean fields; a reference that does not resolve clears the field the same way an absent lookup does.
pub fn resolve_overrides_os_environ<E>(overrides: RedisOverrides, env: &E) -> RedisOverrides
where
    E: EnvironmentLookup,
{
    let passthrough = overrides
        .passthrough
        .into_iter()
        .filter_map(|(name, value)| {
            resolve_passthrough(value, env).map(|resolved| (name, resolved))
        })
        .collect();
    RedisOverrides {
        host: resolve_string(overrides.host, env),
        port: resolve_int(overrides.port, env),
        db: resolve_int(overrides.db, env),
        username: resolve_string(overrides.username, env),
        password: resolve_secret(overrides.password, env),
        url: overrides.url.map(|inner| resolve_secret(inner, env)),
        ssl: resolve_bool(overrides.ssl, env),
        socket_timeout: resolve_float(overrides.socket_timeout, env),
        socket_connect_timeout: resolve_float(overrides.socket_connect_timeout, env),
        socket_keepalive: resolve_bool(overrides.socket_keepalive, env),
        max_connections: resolve_int(overrides.max_connections, env),
        health_check_interval: resolve_int(overrides.health_check_interval, env),
        credential_provider: overrides.credential_provider,
        startup_nodes: resolve_nodes(overrides.startup_nodes, env),
        sentinel_nodes: resolve_nodes(overrides.sentinel_nodes, env),
        sentinel_password: resolve_secret(overrides.sentinel_password, env),
        service_name: resolve_string(overrides.service_name, env),
        ssl_password: resolve_secret(overrides.ssl_password, env),
        gcp_service_account: resolve_string(overrides.gcp_service_account, env),
        gcp_ssl_ca_certs: resolve_string(overrides.gcp_ssl_ca_certs, env),
        azure_redis_ad_token: resolve_secret(overrides.azure_redis_ad_token, env),
        azure_client_id: resolve_string(overrides.azure_client_id, env),
        azure_tenant_id: resolve_string(overrides.azure_tenant_id, env),
        azure_client_secret: resolve_secret(overrides.azure_client_secret, env),
        aws_iam_auth: resolve_bool(overrides.aws_iam_auth, env),
        aws_iam_user_name: resolve_string(overrides.aws_iam_user_name, env),
        aws_iam_cache_name: resolve_string(overrides.aws_iam_cache_name, env),
        aws_iam_region: resolve_string(overrides.aws_iam_region, env),
        aws_iam_serverless: resolve_bool(overrides.aws_iam_serverless, env),
        passthrough,
    }
}

fn resolve_text<E>(text: &str, env: &E) -> Option<String>
where
    E: EnvironmentLookup,
{
    if let Some(name) = text.strip_prefix(PREFIX) {
        return env.lookup(name);
    }
    Some(text.to_owned())
}

fn resolve_string<E>(value: Option<String>, env: &E) -> Option<String>
where
    E: EnvironmentLookup,
{
    let text = value?;
    resolve_text(&text, env)
}

fn resolve_secret<E>(value: Option<SecretString>, env: &E) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    let secret = value?;
    resolve_text(secret.expose_secret(), env).map(SecretString::from)
}

fn resolve_int<E>(value: Option<IntOrText>, env: &E) -> Option<IntOrText>
where
    E: EnvironmentLookup,
{
    match value {
        Some(IntOrText::Text(text)) => resolve_text(&text, env).map(IntOrText::Text),
        other => other,
    }
}

fn resolve_float<E>(value: Option<FloatOrText>, env: &E) -> Option<FloatOrText>
where
    E: EnvironmentLookup,
{
    match value {
        Some(FloatOrText::Text(text)) => resolve_text(&text, env).map(FloatOrText::Text),
        other => other,
    }
}

fn resolve_bool<E>(value: Option<BoolOrText>, env: &E) -> Option<BoolOrText>
where
    E: EnvironmentLookup,
{
    match value {
        Some(BoolOrText::Text(text)) => resolve_text(&text, env).map(BoolOrText::Text),
        other => other,
    }
}

fn resolve_nodes<T, E>(value: Option<NodesInput<T>>, env: &E) -> Option<NodesInput<T>>
where
    E: EnvironmentLookup,
{
    match value {
        Some(NodesInput::Text(text)) => resolve_text(&text, env).map(NodesInput::Text),
        other => other,
    }
}

fn resolve_passthrough<E>(value: PassthroughValue, env: &E) -> Option<PassthroughValue>
where
    E: EnvironmentLookup,
{
    match value {
        PassthroughValue::Text(text) => resolve_text(&text, env).map(PassthroughValue::Text),
        other @ (PassthroughValue::Bool(_)
        | PassthroughValue::Number(_)
        | PassthroughValue::List(_)
        | PassthroughValue::Map(_)) => Some(other),
    }
}
