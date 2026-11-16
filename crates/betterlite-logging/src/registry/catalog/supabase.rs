mod tests;

use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::callback_settings::names::CallbackName;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::supabase::client::SupabaseClient;
use crate::supabase::supabase_logging::SupabaseLogger;
use crate::supabase::supabase_logging::callback::SupabaseCallback;
use crate::supabase::supabase_logging::settings::SupabaseSettings;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let logger = build_callback(inputs)?;
    let callback: Arc<dyn CallbackLogger> = logger;
    Ok(callback)
}

// The same callback is the terminal logger and the pre-call InputEventSink for input_callback.
pub fn build_callback(
    inputs: &CatalogInputs,
) -> Result<Arc<SupabaseCallback<PooledDelivery>>, CatalogError> {
    let environment: &dyn EnvironmentLookup = inputs.env.as_ref();
    let mut settings =
        SupabaseSettings::from_environment(environment).map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })?;
    if let Some(table_name) = table_override(inputs) {
        settings.table_name = table_name;
    }
    Ok(Arc::new(SupabaseCallback {
        logger: Arc::new(SupabaseLogger {
            client: SupabaseClient {
                base_url: settings.url,
                key: settings.key,
                delivery: PooledDelivery {
                    client: inputs.http.clone(),
                },
            },
            table_name: settings.table_name,
        }),
        tasks: inputs.tracker.clone(),
    }))
}

// modify_integration stores the override on the class; the callback_settings entry carries it here
fn table_override(inputs: &CatalogInputs) -> Option<String> {
    let entry = inputs
        .callback_settings
        .get(CallbackName::Supabase.as_str())?;
    entry.exposed_variables().get("table_name").cloned()
}
