use super::client::error::IbmClientError;
use super::config::IbmDetectorConfig;
use super::threshold::filter_detections;
use super::wire::{Detection, ServerDetections};
use crate::content_filter::risk_scoring::GuardrailStatus;

mod tests;

#[must_use]
pub fn detector_server_status(
    config: &IbmDetectorConfig,
    outcome: &Result<ServerDetections, IbmClientError>,
) -> GuardrailStatus {
    let Ok(per_message) = outcome else {
        return GuardrailStatus::GuardrailFailedToRespond;
    };
    if per_message
        .iter()
        .any(|detections| !filter_detections(config, detections).is_empty())
    {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::Success
    }
}

#[must_use]
pub fn orchestrator_status(
    config: &IbmDetectorConfig,
    outcome: &Result<Vec<Detection>, IbmClientError>,
) -> GuardrailStatus {
    match outcome {
        Err(_) => GuardrailStatus::GuardrailFailedToRespond,
        Ok(detections) if filter_detections(config, detections).is_empty() => {
            GuardrailStatus::Success
        }
        Ok(_) => GuardrailStatus::GuardrailIntervened,
    }
}
