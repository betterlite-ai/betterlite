use betterlite_config::loader::EnvironmentLookup;
use serde::Serialize;

use crate::otel::integrations::levo_settings::LevoConfig;

pub const CALLBACK_NAME: &str = "levo";
pub const HEALTHY_MESSAGE: &str = "Levo credentials are configured properly";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum LevoHealth {
    Healthy { message: &'static str },
    Unhealthy { error_message: String },
}

#[must_use]
pub fn health_check(environment: &dyn EnvironmentLookup) -> LevoHealth {
    // get_levo_config validates all four variables and its ValueError text reaches the health answer
    match LevoConfig::from_env(environment) {
        Ok(_) => LevoHealth::Healthy {
            message: HEALTHY_MESSAGE,
        },
        Err(error) => LevoHealth::Unhealthy {
            error_message: error.to_string(),
        },
    }
}

mod tests;
