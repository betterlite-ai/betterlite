use betterlite_config::sections::capability_forecast as section;
use betterlite_router::capability_forecast::config::{Calibration, Config, ResponseFormat};
use betterlite_router::capability_forecast::error::ForecastError;

mod tests;

pub fn router_config(configured: &section::Config) -> Result<Config, ForecastError> {
    let response_format = match configured.response_format {
        section::ResponseFormat::JsonSchema => ResponseFormat::JsonSchema,
        section::ResponseFormat::JsonObject => ResponseFormat::JsonObject,
    };
    Config {
        efficient_tier: configured.efficient_tier.clone(),
        capable_tier: configured.capable_tier.clone(),
        base_threshold: configured.base_threshold,
        threshold_step: configured.threshold_step,
        max_output_tokens: configured.max_output_tokens,
        calibration: configured
            .calibration
            .as_ref()
            .map(|calibration| Calibration {
                version: calibration.version.clone(),
                slope: calibration.slope,
                intercept: calibration.intercept,
            }),
        response_format,
    }
    .validate()
}

pub fn deployment_configs(
    settings: &section::Settings,
) -> Result<Vec<Option<Config>>, ForecastError> {
    settings
        .deployments
        .iter()
        .map(|deployment| deployment.as_ref().map(router_config).transpose())
        .collect()
}
