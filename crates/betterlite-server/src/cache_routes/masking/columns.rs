use betterlite_models::decisions::Json;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;

use super::kwarg_value::KwargValue;

// RedisCache's default when socket_timeout is not passed.
const DEFAULT_SOCKET_TIMEOUT_SECONDS: f64 = 5.0;

type Column = (&'static str, fn(&RedisOverrides) -> Option<Json>);

// LiteLLM's redis_kwargs in its insertion order: the named RedisCache arguments, then the extra kwargs.
pub(super) const COLUMNS: [Column; 28] = [
    ("host", |overrides| {
        overrides.host.as_ref().map(KwargValue::kwarg)
    }),
    ("port", |overrides| {
        overrides.port.as_ref().map(KwargValue::kwarg)
    }),
    ("password", |overrides| {
        overrides
            .password
            .as_ref()
            .and_then(Option::as_ref)
            .map(KwargValue::kwarg)
    }),
    ("startup_nodes", |overrides| {
        overrides.startup_nodes.as_ref().map(KwargValue::kwarg)
    }),
    ("socket_timeout", |overrides| {
        Some(
            overrides
                .socket_timeout
                .as_ref()
                .map_or_else(|| DEFAULT_SOCKET_TIMEOUT_SECONDS.kwarg(), KwargValue::kwarg),
        )
    }),
    ("url", |overrides| {
        overrides
            .url
            .as_ref()
            .and_then(Option::as_ref)
            .map(KwargValue::kwarg)
    }),
    ("db", |overrides| {
        overrides.db.as_ref().map(KwargValue::kwarg)
    }),
    ("username", |overrides| {
        overrides.username.as_ref().map(KwargValue::kwarg)
    }),
    ("ssl", |overrides| {
        overrides.ssl.as_ref().map(KwargValue::kwarg)
    }),
    ("socket_connect_timeout", |overrides| {
        overrides
            .socket_connect_timeout
            .as_ref()
            .map(KwargValue::kwarg)
    }),
    ("socket_keepalive", |overrides| {
        overrides.socket_keepalive.as_ref().map(KwargValue::kwarg)
    }),
    ("max_connections", |overrides| {
        overrides.max_connections.as_ref().map(KwargValue::kwarg)
    }),
    ("health_check_interval", |overrides| {
        overrides
            .health_check_interval
            .as_ref()
            .map(KwargValue::kwarg)
    }),
    ("sentinel_nodes", |overrides| {
        overrides.sentinel_nodes.as_ref().map(KwargValue::kwarg)
    }),
    ("sentinel_password", |overrides| {
        overrides.sentinel_password.as_ref().map(KwargValue::kwarg)
    }),
    ("service_name", |overrides| {
        overrides.service_name.as_ref().map(KwargValue::kwarg)
    }),
    ("ssl_password", |overrides| {
        overrides.ssl_password.as_ref().map(KwargValue::kwarg)
    }),
    ("gcp_service_account", |overrides| {
        overrides
            .gcp_service_account
            .as_ref()
            .map(KwargValue::kwarg)
    }),
    ("gcp_ssl_ca_certs", |overrides| {
        overrides.gcp_ssl_ca_certs.as_ref().map(KwargValue::kwarg)
    }),
    ("azure_redis_ad_token", |overrides| {
        overrides
            .azure_redis_ad_token
            .as_ref()
            .map(KwargValue::kwarg)
    }),
    ("azure_client_id", |overrides| {
        overrides.azure_client_id.as_ref().map(KwargValue::kwarg)
    }),
    ("azure_tenant_id", |overrides| {
        overrides.azure_tenant_id.as_ref().map(KwargValue::kwarg)
    }),
    ("azure_client_secret", |overrides| {
        overrides
            .azure_client_secret
            .as_ref()
            .map(KwargValue::kwarg)
    }),
    ("aws_iam_auth", |overrides| {
        overrides.aws_iam_auth.as_ref().map(KwargValue::kwarg)
    }),
    ("aws_iam_user_name", |overrides| {
        overrides.aws_iam_user_name.as_ref().map(KwargValue::kwarg)
    }),
    ("aws_iam_cache_name", |overrides| {
        overrides.aws_iam_cache_name.as_ref().map(KwargValue::kwarg)
    }),
    ("aws_iam_region", |overrides| {
        overrides.aws_iam_region.as_ref().map(KwargValue::kwarg)
    }),
    ("aws_iam_serverless", |overrides| {
        overrides.aws_iam_serverless.as_ref().map(KwargValue::kwarg)
    }),
];
