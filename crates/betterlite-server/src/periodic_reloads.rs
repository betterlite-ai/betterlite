pub mod beta_headers;
pub mod config_rows;
pub mod cost_map;
pub mod sources;
pub mod state;

mod tests;

use std::sync::Arc;

use betterlite_config::sections::settings::SettingsStore;
use betterlite_error::proxy_enums::supported_db_object_type::SupportedDbObjectType;

use crate::periodic_reloads::config_rows::ReloadConfigRows;
use crate::periodic_reloads::sources::ReloadSources;
use crate::periodic_reloads::state::PeriodicReloads;
use crate::proxy_config_state::supported_db_objects::should_load_db_object;

// LiteLLM hangs the pod reload state on the proxy object and takes prisma_client per call
pub struct PeriodicReloadChecks {
    pub reloads: Arc<PeriodicReloads>,
    pub sources: ReloadSources,
}

impl PeriodicReloadChecks {
    pub async fn check_periodic_reloads<Rows>(&self, db: &Rows, general_settings: &SettingsStore)
    where
        Rows: ReloadConfigRows,
    {
        if !should_load_db_object(
            general_settings,
            &SupportedDbObjectType::ModelCostMap.to_string(),
        ) {
            return;
        }
        if let Err(error) = cost_map::reload(db, &self.reloads, &self.sources).await {
            tracing::error!("Error in _check_and_reload_model_cost_map: {error}");
        }
    }

    pub async fn check_beta_headers_reload<Rows>(&self, db: &Rows)
    where
        Rows: ReloadConfigRows,
    {
        // the check samples the clock once, so the due test and the stored reload time share it
        let now = self.sources.clock.utc_now();
        if let Err(error) = beta_headers::reload(db, &self.reloads, &self.sources, now).await {
            tracing::error!("Error in _check_and_reload_anthropic_beta_headers: {error}");
        }
    }
}
