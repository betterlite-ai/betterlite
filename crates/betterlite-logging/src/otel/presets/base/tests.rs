#![cfg(test)]

use super::PresetError;
use crate::otel::integrations::levo_settings::LevoSettingsError;
use crate::otel::integrations::phoenix_settings::PhoenixSettingsError;
use crate::otel::integrations::weave_settings::WeaveSettingsError;

#[test]
fn preset_error_keeps_the_vendor_message() {
    assert_eq!(
        PresetError::from(LevoSettingsError::CollectorUrl).to_string(),
        "LEVOAI_COLLECTOR_URL environment variable is required for Levo integration. Please contact Levo support to get your collector URL."
    );
    assert_eq!(
        PresetError::from(PhoenixSettingsError::CloudApiKeyMissing).to_string(),
        "PHOENIX_API_KEY must be set when using Phoenix Cloud (app.phoenix.arize.com)."
    );
    assert_eq!(
        PresetError::from(WeaveSettingsError::ProjectId).to_string(),
        "WANDB_PROJECT_ID must be set for Weave OpenTelemetry integration. Format: <entity>/<project_name>"
    );
}
