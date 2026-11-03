mod tests;

use secrecy::ExposeSecret;

use super::error::FactoryError;
use super::fields::ConnectionFields;
use super::{RedisClient, cluster, sentinel, url};
use crate::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings, Target,
};
use crate::redis_connection_settings::target::resolve_target;

// litellm._redis.get_redis_client/get_redis_async_client dispatch; only the async Cluster branch differs.
pub fn build_client(
    settings: &RedisConnectionSettings,
    kind: ClientKind,
) -> Result<RedisClient, FactoryError> {
    let target = resolve_target(
        settings.startup_nodes.as_deref(),
        settings.sentinel_nodes.as_deref(),
        settings.url.as_ref().map(ExposeSecret::expose_secret),
        settings.host.as_deref(),
    )?;
    let use_provider_auth = settings.credential_provider_settings.is_some()
        && (kind == ClientKind::Async || target == Target::Cluster);
    let auth_settings = if use_provider_auth {
        credential_provider_auth(settings)
    } else {
        settings.clone()
    };
    match target {
        Target::Cluster => cluster::build(&auth_settings, kind).map(RedisClient::Cluster),
        Target::Url => url::build(&auth_settings).map(RedisClient::Url),
        Target::Sentinel => sentinel::build(&auth_settings).map(RedisClient::Sentinel),
        Target::Direct => Ok(RedisClient::Direct(ConnectionFields::from(&auth_settings))),
    }
}

pub(super) fn credential_provider_auth(
    settings: &RedisConnectionSettings,
) -> RedisConnectionSettings {
    let mut auth_settings = settings.clone();
    if auth_settings.credential_provider_settings.is_some() {
        auth_settings.credential_provider = true;
        auth_settings.username = None;
        auth_settings.password = None;
        auth_settings.passthrough.remove("redis_connect_func");
    }
    auth_settings
}
