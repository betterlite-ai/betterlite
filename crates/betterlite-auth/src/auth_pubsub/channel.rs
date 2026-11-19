pub const INVALIDATION_CHANNEL: &str = "litellm_proxy.auth_cache_invalidation";

#[must_use]
pub fn invalidation_channel(namespace: Option<&str>) -> String {
    namespace.map_or_else(
        || INVALIDATION_CHANNEL.to_owned(),
        |prefix| format!("{prefix}:{INVALIDATION_CHANNEL}"),
    )
}
