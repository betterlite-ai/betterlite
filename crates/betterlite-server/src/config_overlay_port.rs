use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;

use crate::proxy_config_state::snapshot::ConfigWithBaseline;

// The adapter logs the cause, so a route only needs to know which answer to send
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigOverlayPortError {
    #[error("the config database is not reachable")]
    DatabaseUnavailable,
    #[error("the config could not be reloaded")]
    Reload,
    #[error("the pass-through endpoints could not be served")]
    PassThrough,
}

// B8379's config reload and pass-through re-serve live in the binary crate, so the settings routes reach them through this port
#[async_trait]
pub trait ConfigOverlayPort: Send + Sync {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError>;

    async fn serve_pass_through_endpoints(
        &self,
        db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError>;
}
