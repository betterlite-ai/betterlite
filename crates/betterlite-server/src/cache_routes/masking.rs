use betterlite_models::decisions::{Extra, Json};
use betterlite_redact::structured::SensitiveDataMasker;
use betterlite_redis::redis_connection_settings::model::nodes::{
    ClusterNodeInput, SentinelNodeInput,
};
use betterlite_redis::redis_connection_settings::model::scalars::{
    PassthroughScalar, PassthroughValue, Scalar,
};
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use url::Url;

// RedisCache's default when socket_timeout is not passed.
const DEFAULT_SOCKET_TIMEOUT_SECONDS: f64 = 5.0;

trait KwargValue {
    fn kwarg(&self) -> Json;
}

impl KwargValue for String {
    fn kwarg(&self) -> Json {
        Json::Text(self.clone())
    }
}

impl KwargValue for bool {
    fn kwarg(&self) -> Json {
        Json::Bool(*self)
    }
}

impl KwargValue for i64 {
    fn kwarg(&self) -> Json {
        Json::Number(Number::from(*self))
    }
}

// JSON has no NaN or infinity, so those keep their text.
impl KwargValue for f64 {
    fn kwarg(&self) -> Json {
        Number::from_f64(*self).map_or_else(|| Json::Text(self.to_string()), Json::Number)
    }
}

impl KwargValue for SecretString {
    fn kwarg(&self) -> Json {
        Json::Text(self.expose_secret().to_owned())
    }
}

impl<T: KwargValue> KwargValue for Scalar<T> {
    fn kwarg(&self) -> Json {
        match self {
            Self::Value(value) => value.kwarg(),
            Self::Text(text) => Json::Text(text.clone()),
        }
    }
}

impl KwargValue for Vec<ClusterNodeInput> {
    fn kwarg(&self) -> Json {
        Json::List(
            self.iter()
                .map(|node| {
                    let mut fields = Extra::new();
                    fields.insert("host".to_owned(), node.host.kwarg());
                    fields.insert("port".to_owned(), node.port.kwarg());
                    Json::Object(fields)
                })
                .collect(),
        )
    }
}

impl KwargValue for Vec<SentinelNodeInput> {
    fn kwarg(&self) -> Json {
        Json::List(
            self.iter()
                .map(|node| {
                    let SentinelNodeInput::Tuple((host, port)) = node;
                    Json::List(vec![host.kwarg(), port.kwarg()])
                })
                .collect(),
        )
    }
}

impl KwargValue for PassthroughScalar {
    fn kwarg(&self) -> Json {
        match self {
            Self::Bool(flag) => Json::Bool(*flag),
            Self::Number(number) => Json::Number(number.clone()),
            Self::Text(text) => Json::Text(text.clone()),
        }
    }
}

impl KwargValue for PassthroughValue {
    fn kwarg(&self) -> Json {
        match self {
            Self::Bool(flag) => Json::Bool(*flag),
            Self::Number(number) => Json::Number(number.clone()),
            Self::Text(text) => Json::Text(text.clone()),
            Self::List(items) => Json::List(items.iter().map(KwargValue::kwarg).collect()),
            Self::Map(fields) => Json::Object(
                fields
                    .iter()
                    .map(|(name, value)| (name.clone(), value.kwarg()))
                    .collect(),
            ),
        }
    }
}

fn put<T>(fields: &mut Extra, name: &str, value: Option<&T>)
where
    T: KwargValue,
{
    if let Some(present) = value {
        fields.insert(name.to_owned(), present.kwarg());
    }
}

// LiteLLM's RedisCache.redis_kwargs, masked: the kwargs the cache was built with, before any REDIS_* variable is read.
#[must_use]
pub fn redis_kwargs(overrides: &RedisOverrides) -> Extra {
    let mut fields = Extra::new();
    put(&mut fields, "host", overrides.host.as_ref());
    put(&mut fields, "port", overrides.port.as_ref());
    put(
        &mut fields,
        "password",
        overrides.password.as_ref().and_then(Option::as_ref),
    );
    put(
        &mut fields,
        "startup_nodes",
        overrides.startup_nodes.as_ref(),
    );
    let socket_timeout = overrides
        .socket_timeout
        .clone()
        .unwrap_or(Scalar::Value(DEFAULT_SOCKET_TIMEOUT_SECONDS));
    put(&mut fields, "socket_timeout", Some(&socket_timeout));
    add_connection_kwargs(&mut fields, overrides);
    add_cloud_kwargs(&mut fields, overrides);
    for (name, value) in &overrides.passthrough {
        fields.insert(name.clone(), value.kwarg());
    }
    mask_object(&SensitiveDataMasker::default(), &mut fields);
    fields
}

fn add_connection_kwargs(fields: &mut Extra, overrides: &RedisOverrides) {
    put(
        fields,
        "url",
        overrides.url.as_ref().and_then(Option::as_ref),
    );
    put(fields, "db", overrides.db.as_ref());
    put(fields, "username", overrides.username.as_ref());
    put(fields, "ssl", overrides.ssl.as_ref());
    put(
        fields,
        "socket_connect_timeout",
        overrides.socket_connect_timeout.as_ref(),
    );
    put(
        fields,
        "socket_keepalive",
        overrides.socket_keepalive.as_ref(),
    );
    put(
        fields,
        "max_connections",
        overrides.max_connections.as_ref(),
    );
    put(
        fields,
        "health_check_interval",
        overrides.health_check_interval.as_ref(),
    );
    put(fields, "sentinel_nodes", overrides.sentinel_nodes.as_ref());
    put(
        fields,
        "sentinel_password",
        overrides.sentinel_password.as_ref(),
    );
    put(fields, "service_name", overrides.service_name.as_ref());
    put(fields, "ssl_password", overrides.ssl_password.as_ref());
}

fn add_cloud_kwargs(fields: &mut Extra, overrides: &RedisOverrides) {
    put(
        fields,
        "gcp_service_account",
        overrides.gcp_service_account.as_ref(),
    );
    put(
        fields,
        "gcp_ssl_ca_certs",
        overrides.gcp_ssl_ca_certs.as_ref(),
    );
    put(
        fields,
        "azure_redis_ad_token",
        overrides.azure_redis_ad_token.as_ref(),
    );
    put(
        fields,
        "azure_client_id",
        overrides.azure_client_id.as_ref(),
    );
    put(
        fields,
        "azure_tenant_id",
        overrides.azure_tenant_id.as_ref(),
    );
    put(
        fields,
        "azure_client_secret",
        overrides.azure_client_secret.as_ref(),
    );
    add_aws_kwargs(fields, overrides);
}

fn add_aws_kwargs(fields: &mut Extra, overrides: &RedisOverrides) {
    put(fields, "aws_iam_auth", overrides.aws_iam_auth.as_ref());
    put(
        fields,
        "aws_iam_user_name",
        overrides.aws_iam_user_name.as_ref(),
    );
    put(
        fields,
        "aws_iam_cache_name",
        overrides.aws_iam_cache_name.as_ref(),
    );
    put(fields, "aws_iam_region", overrides.aws_iam_region.as_ref());
    put(
        fields,
        "aws_iam_serverless",
        overrides.aws_iam_serverless.as_ref(),
    );
}

// SensitiveDataMasker.mask_dict's walk over a typed object: mask_dict itself takes serde_json::Map, which the workspace disallows.
fn mask_object(masker: &SensitiveDataMasker, object: &mut Extra) {
    for (key, value) in object.iter_mut() {
        if key == "url" {
            redact_url(masker, value);
        }
        mask_node(masker, value, masker.is_sensitive_key(key, &[]));
    }
}

fn mask_node(masker: &SensitiveDataMasker, value: &mut Json, sensitive: bool) {
    match value {
        Json::Object(inner) => mask_object(masker, inner),
        Json::List(items) => {
            for item in items {
                mask_node(masker, item, sensitive);
            }
        }
        Json::Text(text) if sensitive => *text = masker.mask_value(text),
        Json::Number(number) if sensitive => {
            *value = Json::Text(masker.mask_value(&number.to_string()));
        }
        Json::Bool(flag) if sensitive => {
            *value = Json::Text(masker.mask_value(&flag.to_string()));
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

// Deviation: LiteLLM prints url as given, password included; the userinfo password and the sensitive query values are masked here.
fn redact_url(masker: &SensitiveDataMasker, value: &mut Json) {
    let Json::Text(text) = value else {
        return;
    };
    let Ok(mut parsed) = Url::parse(text) else {
        *text = masker.mask_value(text);
        return;
    };
    if let Some(password) = parsed.password().map(str::to_owned) {
        let hidden = masker.mask_value(&password);
        if parsed.set_password(Some(&hidden)).is_err() {
            *text = masker.mask_value(text);
            return;
        }
    }
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(name, pair_value)| {
            let shown = if masker.is_sensitive_key(&name, &[]) {
                masker.mask_value(&pair_value)
            } else {
                pair_value.into_owned()
            };
            (name.into_owned(), shown)
        })
        .collect();
    if !pairs.is_empty() {
        parsed.query_pairs_mut().clear().extend_pairs(pairs);
    }
    *text = parsed.to_string();
}

mod tests;
