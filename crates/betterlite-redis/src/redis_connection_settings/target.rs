mod tests;

use super::error::RedisSettingsError;
use super::model::nodes::ClusterNodeConfig;
use super::model::settings::Target;

// litellm._redis checks startup_nodes and sentinel_nodes before url, then host last.
pub fn resolve_target(
    startup_nodes: Option<&[ClusterNodeConfig]>,
    sentinel_nodes: Option<&[(String, i64)]>,
    url: Option<&str>,
    host: Option<&str>,
) -> Result<Target, RedisSettingsError> {
    if startup_nodes.is_some_and(|nodes| !nodes.is_empty()) {
        return Ok(Target::Cluster);
    }
    if sentinel_nodes.is_some_and(|nodes| !nodes.is_empty()) {
        return Ok(Target::Sentinel);
    }
    if url.is_some() {
        return Ok(Target::Url);
    }
    if host.is_some() {
        return Ok(Target::Direct);
    }
    Err(RedisSettingsError::MissingTarget)
}
