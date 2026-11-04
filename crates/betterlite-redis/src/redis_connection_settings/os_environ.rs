mod tests;

use secrecy::{ExposeSecret as _, SecretString};

use super::environment::EnvironmentLookup;
use super::model::scalars::{PassthroughValue, Scalar};
use super::model::settings::RedisOverrides;

const PREFIX: &str = "os.environ/";

// litellm._redis._get_redis_client_logic resolves every "os.environ/<name>" string in env_overrides through get_secret before merging with the environment, parsing startup or sentinel nodes, or coercing numeric and boolean fields; a reference that does not resolve clears the field the same way an absent lookup does.
pub fn resolve_overrides_os_environ<E>(mut overrides: RedisOverrides, env: &E) -> RedisOverrides
where
    E: EnvironmentLookup,
{
    overrides.passthrough = overrides
        .passthrough
        .into_iter()
        .filter_map(|(name, value)| {
            resolve_textual(Some(value), env).map(|resolved| (name, resolved))
        })
        .collect();
    resolve_string_references(&mut overrides, env);
    for field in [&mut overrides.password, &mut overrides.url] {
        *field = field
            .take()
            .map(|secret| resolve_text(secret?.expose_secret(), env).map(SecretString::from));
    }
    for field in [
        &mut overrides.port,
        &mut overrides.db,
        &mut overrides.max_connections,
        &mut overrides.health_check_interval,
    ] {
        *field = resolve_textual(field.take(), env);
    }
    for field in [
        &mut overrides.ssl,
        &mut overrides.socket_keepalive,
        &mut overrides.aws_iam_auth,
        &mut overrides.aws_iam_serverless,
    ] {
        *field = resolve_textual(field.take(), env);
    }
    for field in [
        &mut overrides.socket_timeout,
        &mut overrides.socket_connect_timeout,
    ] {
        *field = resolve_textual(field.take(), env);
    }
    overrides.startup_nodes = resolve_textual(overrides.startup_nodes.take(), env);
    overrides.sentinel_nodes = resolve_textual(overrides.sentinel_nodes.take(), env);
    overrides
}

fn resolve_string_references<E>(overrides: &mut RedisOverrides, env: &E)
where
    E: EnvironmentLookup,
{
    for field in [
        &mut overrides.host,
        &mut overrides.username,
        &mut overrides.service_name,
        &mut overrides.gcp_service_account,
        &mut overrides.gcp_ssl_ca_certs,
        &mut overrides.azure_client_id,
        &mut overrides.azure_tenant_id,
        &mut overrides.aws_iam_user_name,
        &mut overrides.aws_iam_cache_name,
        &mut overrides.aws_iam_region,
    ] {
        *field = field.take().and_then(|text| resolve_text(&text, env));
    }
    for field in [
        &mut overrides.sentinel_password,
        &mut overrides.ssl_password,
        &mut overrides.azure_redis_ad_token,
        &mut overrides.azure_client_secret,
    ] {
        *field = field
            .take()
            .and_then(|secret| resolve_text(secret.expose_secret(), env).map(SecretString::from));
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
