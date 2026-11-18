pub const INVALIDATION_CHANNEL: &str = "litellm_proxy.auth_cache_invalidation";

#[must_use]
pub fn invalidation_channel(namespace: Option<&str>) -> String {
    match namespace {
        None => INVALIDATION_CHANNEL.to_owned(),
        Some(namespace) => format!("{namespace}:{INVALIDATION_CHANNEL}"),
    }
}
