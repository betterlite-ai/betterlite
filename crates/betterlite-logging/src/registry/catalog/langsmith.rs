use std::sync::{Arc, Mutex};

use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};

use super::{CatalogError, CatalogInputs};
use crate::argilla_logging::logger::SeededRandom;
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::langsmith_logger::clock::SystemClock;
use crate::langsmith_logger::settings::{self, LangsmithSettings};
use crate::langsmith_logger::{LangsmithDeps, LangsmithLogger};
use crate::langsmith_payloads::ids::SystemIds;

mod tests;

pub fn resolve_settings(
    inputs: &CatalogInputs,
) -> Result<LangsmithSettings, settings::SettingsError> {
    settings::resolve(
        inputs.env.as_ref(),
        inputs.config.sections.langsmith_settings.batch_size,
        inputs
            .config
            .sections
            .logging_privacy
            .redact_user_api_key_info,
    )
}

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings = resolve_settings(inputs).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let random = StdRng::try_from_rng(&mut SysRng).map_err(|source| CatalogError::Init {
        message: format!("the LangSmith sampling source is unavailable: {source}"),
    })?;
    Ok(LangsmithLogger::new(
        settings,
        LangsmithDeps {
            delivery: PooledDelivery {
                client: inputs.http.clone(),
            },
            environment: Arc::clone(&inputs.env),
            random: SeededRandom {
                generator: Mutex::new(random),
            },
            ids: Arc::new(SystemIds),
            clock: Arc::new(SystemClock),
            tasks: inputs.tracker.clone(),
            shutdown: inputs.shutdown.clone(),
        },
    ))
}
