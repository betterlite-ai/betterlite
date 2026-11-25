use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::{includes::IncludeError, settings::JsonValue};
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_server::bucket_config::error::BucketConfigError;
use betterlite_server::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use betterlite_server::proxy_config_state::snapshot::ConfigWithBaseline;
use tokio::sync::Mutex;

use super::environ::Workers;
use super::error::OverlayError;
use super::pass_through::{
    PassThroughInstaller, install_pass_through_endpoints, publish_pass_through,
    validate_db_endpoints,
};
use super::reload::{begin_reload, finish_reload, reload_stores, resolve_reload};
use super::rows::fetch_db_rows;
use super::source::{ConfigSource, read_config};
use super::state::{OverlayDatabase, OverlayState};

// The two functions' arguments, the off-runtime thread owner, and the lock that keeps a reload and a serve from interleaving.
pub struct OverlayPortAdapter {
    pub state: Arc<Mutex<OverlayState>>,
    pub source: ConfigSource,
    pub database: Option<OverlayDatabase>,
    pub environment: Arc<Mutex<BTreeMap<String, String>>>,
    pub installer: Arc<dyn PassThroughInstaller>,
    pub workers: Arc<Workers>,
    pub mutations: Mutex<()>,
}

// The mutation lock makes a reload and a serve run one after the other, as LiteLLM's event loop does; no other guard spans an await.
#[async_trait]
impl ConfigOverlayPort for OverlayPortAdapter {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError> {
        let _mutations = self.mutations.lock().await;
        let mut environment = self.environment.lock().await.clone();
        let file_config = read_config(&self.source, &environment, &self.workers)
            .await
            .map_err(|error| port_error(&error, ConfigOverlayPortError::Reload))?;
        let store_model_in_db = {
            let mut state = self.state.lock().await;
            reload_stores(&mut state, &file_config)
                .map_err(|error| port_error(&error, ConfigOverlayPortError::Reload))?;
            state.config.store_model_in_db
        };
        let stored = if let Some(backed) = self.database.as_ref().filter(|_| store_model_in_db) {
            let rows = fetch_db_rows(backed)
                .await
                .map_err(|error| port_error(&error, ConfigOverlayPortError::Reload))?;
            Some((backed, rows))
        } else {
            None
        };
        let pending = {
            let mut state = self.state.lock().await;
            let source = &self.source;
            let env = &mut environment;
            begin_reload(&mut state, &file_config, stored, source, env)
                .map_err(|error| port_error(&error, ConfigOverlayPortError::Reload))?
        };
        self.environment.lock().await.extend(environment);
        let resolved = resolve_reload(pending, &self.workers)
            .await
            .map_err(|error| port_error(&error, ConfigOverlayPortError::Reload))?;
        let mut state = self.state.lock().await;
        Ok(finish_reload(&mut state, resolved))
    }

    async fn serve_pass_through_endpoints(
        &self,
        db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError> {
        let _mutations = self.mutations.lock().await;
        let mut state = self.state.lock().await;
        publish_pass_through(&mut state, db_endpoints)
            .map_err(|error| port_error(&error, ConfigOverlayPortError::PassThrough))?;
        drop(state);
        validate_db_endpoints(db_endpoints)
            .map_err(|error| port_error(&error, ConfigOverlayPortError::PassThrough))?;
        install_pass_through_endpoints(db_endpoints, self.installer.as_ref())
            .await
            .map_err(|error| port_error(&error, ConfigOverlayPortError::PassThrough))
    }
}

// The log keeps the typed cause but never the YAML source excerpt: a parser renders the neighboring lines, and a config file carries keys.
fn port_error(error: &OverlayError, step: ConfigOverlayPortError) -> ConfigOverlayPortError {
    let message = match error {
        OverlayError::ConfigYaml { source, .. } => format!("{error}: {}", source.without_snippet()),
        OverlayError::Includes(IncludeError::Yaml { path, source })
        | OverlayError::Bucket(BucketConfigError::Include(IncludeError::Yaml { path, source })) => {
            format!(
                "invalid YAML in config include {}: {}",
                path.display(),
                source.without_snippet()
            )
        }
        other => other.to_string(),
    };
    tracing::error!(error = %message, "the config overlay failed");
    if let OverlayError::Database { source } = error
        && is_native_transport_error(source)
    {
        return ConfigOverlayPortError::DatabaseUnavailable;
    }
    step
}
