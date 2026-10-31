mod tests;

use secrecy::{ExposeSecret as _, SecretString};

use super::environment::EnvironmentLookup;
use super::model::scalars::{PassthroughValue, Scalar};
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
            resolve_textual(Some(value), env).map(|resolved| (name, resolved))
        })
        .collect();
    RedisOverrides {
        host: resolve_string(overrides.host, env),
        port: resolve_textual(overrides.port, env),
        db: resolve_textual(overrides.db, env),
        username: resolve_string(overrides.username, env),
        password: overrides.password.map(|value| resolve_secret(value, env)),
        url: overrides.url.map(|inner| resolve_secret(inner, env)),
        ssl: resolve_textual(overrides.ssl, env),
        socket_timeout: resolve_textual(overrides.socket_timeout, env),
        socket_connect_timeout: resolve_textual(overrides.socket_connect_timeout, env),
        socket_keepalive: resolve_textual(overrides.socket_keepalive, env),
        max_connections: resolve_textual(overrides.max_connections, env),
        health_check_interval: resolve_textual(overrides.health_check_interval, env),
        credential_provider: overrides.credential_provider,
        startup_nodes: resolve_textual(overrides.startup_nodes, env),
        sentinel_nodes: resolve_textual(overrides.sentinel_nodes, env),
        sentinel_password: resolve_secret(overrides.sentinel_password, env),
        service_name: resolve_string(overrides.service_name, env),
        ssl_password: resolve_secret(overrides.ssl_password, env),
        gcp_service_account: resolve_string(overrides.gcp_service_account, env),
        gcp_ssl_ca_certs: resolve_string(overrides.gcp_ssl_ca_certs, env),
        azure_redis_ad_token: resolve_secret(overrides.azure_redis_ad_token, env),
        azure_client_id: resolve_string(overrides.azure_client_id, env),
        azure_tenant_id: resolve_string(overrides.azure_tenant_id, env),
        azure_client_secret: resolve_secret(overrides.azure_client_secret, env),
        aws_iam_auth: resolve_textual(overrides.aws_iam_auth, env),
        aws_iam_user_name: resolve_string(overrides.aws_iam_user_name, env),
        aws_iam_cache_name: resolve_string(overrides.aws_iam_cache_name, env),
        aws_iam_region: resolve_string(overrides.aws_iam_region, env),
        aws_iam_serverless: resolve_textual(overrides.aws_iam_serverless, env),
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

// IntOrText, FloatOrText, BoolOrText, NodesInput and PassthroughValue each carry a Text variant that may itself be an "os.environ/<name>" reference, so every one of them resolves through the same text-or-leave-as-is rule.
trait TextVariant: Sized {
    fn into_text(self) -> Result<String, Self>;
    fn from_text(text: String) -> Self;
}

impl<T> TextVariant for Scalar<T> {
    fn into_text(self) -> Result<String, Self> {
        match self {
            Self::Text(text) => Ok(text),
            other @ Self::Value(_) => Err(other),
        }
    }

    fn from_text(text: String) -> Self {
        Self::Text(text)
    }
}

impl TextVariant for PassthroughValue {
    fn into_text(self) -> Result<String, Self> {
        match self {
            Self::Text(text) => Ok(text),
            other @ (Self::Bool(_) | Self::Number(_) | Self::List(_) | Self::Map(_)) => Err(other),
        }
    }

    fn from_text(text: String) -> Self {
        Self::Text(text)
    }
}

fn resolve_textual<T, E>(value: Option<T>, env: &E) -> Option<T>
where
    T: TextVariant,
    E: EnvironmentLookup,
{
    match value?.into_text() {
        Ok(text) => resolve_text(&text, env).map(T::from_text),
        Err(variant) => Some(variant),
    }
}
