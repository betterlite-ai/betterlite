use betterlite_config::sections::billing_metrics::Settings;

use crate::active_recorder::ActiveRecorderRegistry;
use crate::meter_provider::{BillingMetricsConfig, build_mtls_meter_provider};
use crate::recorder::BillingMetricsRecorder;

mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LicenseData {
    pub user_id: Option<String>,
}

#[must_use]
pub fn build_recorder(
    premium: bool,
    license_data: Option<&LicenseData>,
    litellm_version: &str,
    settings: &Settings,
    registry: &ActiveRecorderRegistry,
) -> Option<BillingMetricsRecorder> {
    if !premium {
        // unlicensed is the common case, so this one disable path stays at debug
        tracing::debug!("Billing metrics disabled: deployment is not licensed");
        return None;
    }
    let config = BillingMetricsConfig {
        metering: settings.resolve()?,
        litellm_version: litellm_version.to_owned(),
        license_id: license_data.and_then(|license| license.user_id.clone()),
    };
    let provider = match build_mtls_meter_provider(&config) {
        Ok(provider) => provider,
        Err(error) => {
            tracing::warn!("Billing metrics disabled: failed to initialize exporter: {error}");
            return None;
        }
    };
    let recorder = BillingMetricsRecorder::new(provider);
    registry.set(recorder.clone());
    tracing::info!(
        "Billing metrics enabled: exporting to {} every {} ms",
        config.metering.endpoint,
        config.metering.export_interval_ms
    );
    Some(recorder)
}

pub fn shutdown_recorder(registry: &ActiveRecorderRegistry) {
    if let Some(recorder) = registry.pop() {
        recorder.shutdown();
    }
}
