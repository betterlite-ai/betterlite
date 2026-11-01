pub mod cluster;
pub mod dispatch;
pub mod error;
pub mod fields;
pub mod pool;
pub mod redact;
pub mod sentinel;
pub mod url;

// Mirrors litellm._redis.get_redis_client/get_redis_async_client's return type.
#[derive(Debug, Clone)]
pub enum RedisClient {
    Direct(fields::ConnectionFields),
    Url(url::UrlClient),
    Sentinel(sentinel::SentinelClient),
    Cluster(cluster::ClusterClient),
}
