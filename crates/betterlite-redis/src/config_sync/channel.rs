pub const CONFIG_SYNC_CHANNEL: &str = "litellm_proxy.config_change";

#[must_use]
pub fn config_sync_channel(namespace: Option<&str>) -> String {
    namespace.map_or_else(
        || CONFIG_SYNC_CHANNEL.to_owned(),
        |prefix| format!("{prefix}:{CONFIG_SYNC_CHANNEL}"),
    )
}
