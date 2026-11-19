use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::finops::cloudzero_settings::CLOUDZERO_SETTINGS_PARAM_NAME;
use betterlite_db::finops::settings_row::settings_row_present;

pub const CLOUDZERO_CALLBACK: &str = "cloudzero";

// LiteLLM's is_cloudzero_setup_in_config: "cloudzero" in litellm.callbacks, by name only
#[must_use]
pub fn is_cloudzero_setup_in_config(callbacks: &[String]) -> bool {
    callbacks
        .iter()
        .any(|callback| callback.as_str() == CLOUDZERO_CALLBACK)
}

// LiteLLM's is_cloudzero_setup_in_db: a stored row whose value is not None, with every failure reported as false
pub async fn is_cloudzero_setup_in_db(client: Option<PrismaClient<'_>>) -> bool {
    match settings_row_present(client, CLOUDZERO_SETTINGS_PARAM_NAME).await {
        Ok(present) => present,
        Err(error) => {
            tracing::error!(%error, "error checking the CloudZero settings in the database");
            false
        }
    }
}

// LiteLLM's is_cloudzero_setup: the config callbacks answer first and the database is the fallback
pub async fn is_cloudzero_setup(client: Option<PrismaClient<'_>>, callbacks: &[String]) -> bool {
    if is_cloudzero_setup_in_config(callbacks) {
        return true;
    }
    is_cloudzero_setup_in_db(client).await
}
