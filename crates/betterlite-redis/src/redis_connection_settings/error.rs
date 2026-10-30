mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RedisSettingsError {
    #[error("Either 'host' or 'url' must be specified for redis.")]
    MissingTarget,
    #[error("REDIS_CLUSTER_NODES is not valid JSON: {0}")]
    InvalidClusterNodesJson(serde_json::Error),
    #[error("REDIS_SENTINEL_NODES is not valid JSON: {0}")]
    InvalidSentinelNodesJson(serde_json::Error),
    #[error("redis url is not a valid URL: {0}")]
    InvalidUrl(url::ParseError),
    #[error("redis url does not support embedding user info")]
    UrlCannotStripUserinfo,
}
