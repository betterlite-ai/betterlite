mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RedisSettingsError {
    #[error("Either 'host' or 'url' must be specified for redis.")]
    MissingTarget,
    #[error("REDIS_CLUSTER_NODES is not valid JSON: {source}", source = source)]
    InvalidClusterNodesJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("REDIS_SENTINEL_NODES is not valid JSON: {source}", source = source)]
    InvalidSentinelNodesJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("redis url is not a valid URL: {source}", source = source)]
    InvalidUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("redis url does not support embedding user info")]
    UrlCannotStripUserinfo,
}
