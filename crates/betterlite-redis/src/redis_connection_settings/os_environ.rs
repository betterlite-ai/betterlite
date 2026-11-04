mod tests;

use std::collections::BTreeMap;
use std::mem;

use secrecy::{ExposeSecret as _, SecretString};

use super::environment::EnvironmentLookup;
use super::model::scalars::{PassthroughValue, Scalar};
use super::model::settings::RedisOverrides;

const PREFIX: &str = "os.environ/";

// A setting whose text came from an "os.environ/<name>" reference blames that name, not the setting's own REDIS_* variable, once coercion fails; keyed by the canonical name resolve.rs and iam.rs already pass to Coerce::coerce, coerce_bool and coerce_passthrough.
pub(crate) type ReferenceNames = BTreeMap<String, String>;

// litellm._redis._get_redis_client_logic resolves every "os.environ/<name>" string in env_overrides through get_secret before merging with the environment, parsing startup or sentinel nodes, or coercing numeric and boolean fields; a reference that does not resolve clears the field the same way an absent lookup does.
pub fn resolve_overrides_os_environ<E>(
    mut overrides: RedisOverrides,
    env: &E,
) -> (RedisOverrides, ReferenceNames)
where
    E: EnvironmentLookup,
{
    let mut names = ReferenceNames::new();
    resolve_passthrough_references(&mut overrides, env, &mut names);
    resolve_string_references(&mut overrides, env);
    resolve_coercible_fields(&mut overrides, env, &mut names);
    overrides.startup_nodes = resolve_textual(overrides.startup_nodes.take(), env).0;
    overrides.sentinel_nodes = resolve_textual(overrides.sentinel_nodes.take(), env).0;
    (overrides, names)
}

// IntOrText, BoolOrText and FloatOrText each resolve through resolve_textual, but a caller holding three typed Option<Scalar<T>> fields needs one loop over all of them, not a separate monomorphized call per type.
enum CoercibleField<'field> {
    Int(&'field mut Option<Scalar<i64>>),
    Bool(&'field mut Option<Scalar<bool>>),
    Float(&'field mut Option<Scalar<f64>>),
}

impl CoercibleField<'_> {
    fn resolve<E>(self, canonical: &str, env: &E, names: &mut ReferenceNames)
    where
        E: EnvironmentLookup,
    {
        match self {
            Self::Int(field) => *field = resolve_blamed(canonical, field.take(), env, names),
            Self::Bool(field) => *field = resolve_blamed(canonical, field.take(), env, names),
            Self::Float(field) => *field = resolve_blamed(canonical, field.take(), env, names),
        }
    }
}

// Resolves every numeric and boolean field that coercion.rs and resolve.rs later parse with betterlite_env, so a reference's name is recorded before its canonical REDIS_* name is the only one left.
fn resolve_coercible_fields<E>(overrides: &mut RedisOverrides, env: &E, names: &mut ReferenceNames)
where
    E: EnvironmentLookup,
{
    let fields = [
        (CoercibleField::Int(&mut overrides.port), "REDIS_PORT"),
        (CoercibleField::Int(&mut overrides.db), "REDIS_DB"),
        (
            CoercibleField::Int(&mut overrides.max_connections),
            "REDIS_MAX_CONNECTIONS",
        ),
        (
            CoercibleField::Int(&mut overrides.health_check_interval),
            "REDIS_HEALTH_CHECK_INTERVAL",
        ),
        (CoercibleField::Bool(&mut overrides.ssl), "REDIS_SSL"),
        (
            CoercibleField::Bool(&mut overrides.socket_keepalive),
            "REDIS_SOCKET_KEEPALIVE",
        ),
        (
            CoercibleField::Bool(&mut overrides.aws_iam_auth),
            "REDIS_AWS_IAM_AUTH",
        ),
        (
            CoercibleField::Bool(&mut overrides.aws_iam_serverless),
            "REDIS_AWS_IAM_SERVERLESS",
        ),
        (
            CoercibleField::Float(&mut overrides.socket_timeout),
            "REDIS_SOCKET_TIMEOUT",
        ),
        (
            CoercibleField::Float(&mut overrides.socket_connect_timeout),
            "REDIS_SOCKET_CONNECT_TIMEOUT",
        ),
    ];
    for (field, canonical) in fields {
        field.resolve(canonical, env, names);
    }
}

fn resolve_passthrough_references<E>(
    overrides: &mut RedisOverrides,
    env: &E,
    names: &mut ReferenceNames,
) where
    E: EnvironmentLookup,
{
    overrides.passthrough = mem::take(&mut overrides.passthrough)
        .into_iter()
        .filter_map(|(key, value)| {
            let canonical = format!("REDIS_{}", key.to_uppercase());
            resolve_blamed(&canonical, Some(value), env, names).map(|resolved| (key, resolved))
        })
        .collect();
}

// Resolves value through the same "os.environ/<name>" rule as resolve_textual, but also records the referenced name under canonical, so a coercion error later blames CACHE_PORT instead of REDIS_PORT.
fn resolve_blamed<T, E>(
    canonical: &str,
    value: Option<T>,
    env: &E,
    names: &mut ReferenceNames,
) -> Option<T>
where
    T: TextVariant,
    E: EnvironmentLookup,
{
    let (resolved, blamed) = resolve_textual(value, env);
    if let Some(name) = blamed {
        names.insert(canonical.to_owned(), name);
    }
    resolved
}

// None of these fields feed betterlite_env::number or betterlite_env::flag later, so their resolution never needs the referenced name, only the resolved text.
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
        *field = field
            .take()
            .and_then(|text| resolve_named(&text, env).map(|(value, _blamed)| value));
    }
    for field in [
        &mut overrides.sentinel_password,
        &mut overrides.ssl_password,
        &mut overrides.azure_redis_ad_token,
        &mut overrides.azure_client_secret,
    ] {
        *field = field.take().and_then(|secret| {
            resolve_named(secret.expose_secret(), env)
                .map(|(value, _blamed)| value)
                .map(SecretString::from)
        });
    }
    for field in [&mut overrides.password, &mut overrides.url] {
        *field = field.take().map(|secret| {
            resolve_named(secret?.expose_secret(), env)
                .map(|(value, _blamed)| value)
                .map(SecretString::from)
        });
    }
}

// Resolves the same "os.environ/<name>" reference that resolve_string_references and resolve_textual use, but also hands back the name a caller should blame in a coercion error, so a field whose text came from CACHE_PORT never reports REDIS_PORT.
fn resolve_named<E>(text: &str, env: &E) -> Option<(String, Option<String>)>
where
    E: EnvironmentLookup,
{
    text.strip_prefix(PREFIX).map_or_else(
        || Some((text.to_owned(), None)),
        |name| env.lookup(name).map(|value| (value, Some(name.to_owned()))),
    )
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

fn resolve_textual<T, E>(value: Option<T>, env: &E) -> (Option<T>, Option<String>)
where
    T: TextVariant,
    E: EnvironmentLookup,
{
    let Some(scalar) = value else {
        return (None, None);
    };
    match scalar.into_text() {
        Ok(text) => resolve_named(&text, env).map_or((None, None), |(resolved, name)| {
            (Some(T::from_text(resolved)), name)
        }),
        Err(original) => (Some(original), None),
    }
}
