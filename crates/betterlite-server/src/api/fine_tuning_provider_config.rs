use std::sync::{Arc, PoisonError, RwLock};

use betterlite_config::api::fine_tuning_provider_config::{
    FineTuningConfigError, Settings as FineTuningSettings, fine_tuning_config_for,
    set_fine_tuning_config,
};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;

mod tests;

// the finetune_settings snapshot the startup config installs and every route lookup shares
#[derive(Debug, Clone)]
pub struct FineTuningProviderConfig {
    current: Arc<RwLock<Option<Vec<Json>>>>,
}

impl FineTuningProviderConfig {
    #[must_use]
    pub fn new(settings: &FineTuningSettings) -> Self {
        Self {
            current: Arc::new(RwLock::new(settings.finetune_settings.clone())),
        }
    }

    // set_fine_tuning_config: a null config keeps the installed list, a list replaces it
    pub fn set<L>(&self, config: Option<&Json>, lookup: &L) -> Result<(), FineTuningConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        let next = set_fine_tuning_config(current.clone(), config, lookup)?;
        *current = next;
        Ok(())
    }

    // fine_tuning_config_for: the whole first entry whose custom_llm_provider matches
    pub fn provider(&self, provider: &str) -> Result<Option<Json>, FineTuningConfigError> {
        let current = self.current.read().unwrap_or_else(PoisonError::into_inner);
        fine_tuning_config_for(current.as_deref(), provider)
    }
}
