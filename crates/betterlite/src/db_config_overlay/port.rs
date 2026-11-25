use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_server::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use betterlite_server::proxy_config_state::snapshot::ConfigWithBaseline;
use tokio::sync::Mutex;

use super::error::OverlayError;
use super::pass_through::{PassThroughInstaller, serve_pass_through_endpoints};
use super::reload::reload_config;
use super::source::ConfigSource;
use super::state::{OverlayDatabase, OverlayState};

// One field per argument of reload_config and serve_pass_through_endpoints; the locks serialize reloads as LiteLLM's one event loop does
pub struct OverlayPortAdapter {
    pub state: Arc<Mutex<OverlayState>>,
    pub source: ConfigSource,
    pub database: Option<OverlayDatabase>,
    pub environment: Arc<Mutex<BTreeMap<String, String>>>,
    pub installer: Arc<dyn PassThroughInstaller>,
}

#[async_trait]
impl ConfigOverlayPort for OverlayPortAdapter {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError> {
        let mut state = self.state.lock().await;
        let mut environment = self.environment.lock().await;
        reload_config(
            &mut state,
            &self.source,
            self.database.as_ref(),
            &mut environment,
        )
        .await
        .map_err(|error| port_error(&error, ConfigOverlayPortError::Reload))
    }

    async fn serve_pass_through_endpoints(
        &self,
        db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError> {
        let mut state = self.state.lock().await;
        serve_pass_through_endpoints(&mut state, db_endpoints, self.installer.as_ref())
            .await
            .map_err(|error| port_error(&error, ConfigOverlayPortError::PassThrough))
    }
}

// a database read that fails in transport is an outage the routes answer with 503; everything else keeps the step's own error
fn port_error(error: &OverlayError, step: ConfigOverlayPortError) -> ConfigOverlayPortError {
    let cause = error.source().map(ToString::to_string);
    tracing::error!(%error, ?cause, "the config overlay failed");
    if let OverlayError::Database { source } = error
        && is_native_transport_error(source)
    {
        return ConfigOverlayPortError::DatabaseUnavailable;
    }
    step
}
