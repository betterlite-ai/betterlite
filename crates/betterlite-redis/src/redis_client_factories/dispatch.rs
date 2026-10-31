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
    match target {
        Target::Cluster => cluster::build(settings, kind).map(RedisClient::Cluster),
        Target::Url => url::build(settings).map(RedisClient::Url),
        Target::Sentinel => sentinel::build(settings).map(RedisClient::Sentinel),
        Target::Direct => Ok(RedisClient::Direct(ConnectionFields::from(settings))),
    }
}
