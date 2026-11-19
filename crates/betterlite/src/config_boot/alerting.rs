use betterlite_config::sections::settings::JsonValue;
use betterlite_router::prompt_dispatch::known_callbacks::KNOWN_CALLBACKS;

use super::error::ConfigBootError;

const DEFAULT_ALERTING_THRESHOLD: f64 = 600.0;

pub struct AlertingUpdate<'config> {
    pub alerting: &'config [String],
    pub alerting_threshold: Option<f64>,
    pub alert_types: Option<&'config JsonValue>,
    pub alert_to_webhook_url: Option<&'config JsonValue>,
    pub alerting_args: Option<&'config JsonValue>,
    pub alert_type_config: Option<&'config JsonValue>,
}

// B8399 owns update_values and the custom-logger registration behind these methods
pub trait AlertingPort: Send + Sync {
    fn update_alerting(&self, update: &AlertingUpdate<'_>) -> Result<(), ConfigBootError>;

    fn add_alerting_logger(
        &self,
        name: &str,
        alerting_args: Option<&JsonValue>,
    ) -> Result<(), ConfigBootError>;
}

pub fn load_alerting_settings(
    general_settings: &JsonValue,
    ports: &dyn AlertingPort,
) -> Result<(), ConfigBootError> {
    if general_settings.is_null() {
        return Ok(());
    }
    if !general_settings.is_object() {
        return Err(ConfigBootError::FieldType {
            field: "general_settings",
            expected: "a mapping",
        });
    }
    let Some(alerting) = general_settings.get("alerting") else {
        return Ok(());
    };
    if alerting.is_null() {
        return Ok(());
    }
    let alerting = alerting_names(alerting)?;
    let update = AlertingUpdate {
        alerting: &alerting,
        alerting_threshold: alerting_threshold(general_settings.get("alerting_threshold"))?,
        alert_types: general_settings
            .get("alert_types")
            .filter(|value| !value.is_null()),
        alert_to_webhook_url: general_settings
            .get("alert_to_webhook_url")
            .filter(|value| !value.is_null()),
        alerting_args: general_settings
            .get("alerting_args")
            .filter(|value| !value.is_null()),
        alert_type_config: general_settings
            .get("alert_type_config")
            .filter(|value| !value.is_null()),
    };
    ports.update_alerting(&update)?;
    register_loggers(&alerting, update.alerting_args, ports);
    Ok(())
}

fn alerting_names(alerting: &JsonValue) -> Result<Vec<String>, ConfigBootError> {
    serde_json::from_value(alerting.clone()).map_err(|source| ConfigBootError::Step {
        step: "reading general_settings.alerting",
        source: Box::new(source),
    })
}

fn alerting_threshold(configured: Option<&JsonValue>) -> Result<Option<f64>, ConfigBootError> {
    let Some(value) = configured else {
        return Ok(Some(DEFAULT_ALERTING_THRESHOLD));
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value::<f64>(value.clone())
        .map(Some)
        .map_err(|source| ConfigBootError::Step {
            step: "reading general_settings.alerting_threshold",
            source: Box::new(source),
        })
}

fn register_loggers(
    alerting: &[String],
    alerting_args: Option<&JsonValue>,
    ports: &dyn AlertingPort,
) {
    for name in alerting {
        if name == "slack" {
            continue;
        }
        if KNOWN_CALLBACKS.contains(&name.as_str())
            && let Err(error) = ports.add_alerting_logger(name, alerting_args)
        {
            // LiteLLM logs a failing alerting logger and keeps booting
            tracing::error!("[Non-Blocking Error] Error initializing custom logger: {error}");
        }
    }
}

mod tests;
