use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::finops::settings_row_presence::settings_row_present;
use betterlite_db::finops::vantage_settings::VANTAGE_SETTINGS_PARAM_NAME;

use super::vantage::store::VantageCallbackList;
use super::vantage::store::VantageLoggers;

pub const VANTAGE_CALLBACK: &str = "vantage";

// LiteLLM's is_vantage_setup_in_config: the name or a registered VantageLogger in litellm.callbacks
#[must_use]
pub fn is_vantage_setup_in_config(callbacks: &[String], loggers: &VantageLoggers) -> bool {
    callbacks
        .iter()
        .any(|callback| callback.as_str() == VANTAGE_CALLBACK)
        || loggers.has_registered_in(VantageCallbackList::Callbacks)
}

// LiteLLM's is_vantage_setup_in_db: a stored row whose value is not None, with every failure reported as false
pub async fn is_vantage_setup_in_db(client: Option<PrismaClient<'_>>) -> bool {
    match settings_row_present(client, VANTAGE_SETTINGS_PARAM_NAME).await {
        Ok(present) => present,
        Err(error) => {
            tracing::error!(%error, "error checking the Vantage settings in the database");
            false
        }
    }
}

// LiteLLM's is_vantage_setup: the config callbacks answer first and the database is the fallback
pub async fn is_vantage_setup(
    client: Option<PrismaClient<'_>>,
    callbacks: &[String],
    loggers: &VantageLoggers,
) -> bool {
    if is_vantage_setup_in_config(callbacks, loggers) {
        return true;
    }
    is_vantage_setup_in_db(client).await
}
