use super::IbmGuardrail;
use super::error::IbmHookError;
use crate::ibm_detector_client::config::IbmDetectorConfig;
use crate::ibm_detector_client::message::{detector_server_message, orchestrator_message};
use crate::ibm_detector_client::threshold::filter_detections;

pub(crate) async fn check_contents(
    guardrail: &IbmGuardrail,
    contents: &[String],
) -> Result<(), IbmHookError> {
    let config = &guardrail.client.config;
    if config.is_detector_server {
        return check_detector_server(guardrail, config, contents).await;
    }
    for content in contents {
        let detections = guardrail.client.call_orchestrator(content).await?;
        if config.block_on_detection && !filter_detections(config, &detections).is_empty() {
            return Err(IbmHookError::Violation {
                message: orchestrator_message(config, &detections),
            });
        }
    }
    Ok(())
}

async fn check_detector_server(
    guardrail: &IbmGuardrail,
    config: &IbmDetectorConfig,
    contents: &[String],
) -> Result<(), IbmHookError> {
    let per_message = guardrail.client.call_detector_server(contents).await?;
    let violated = per_message
        .iter()
        .any(|detections| !filter_detections(config, detections).is_empty());
    if violated && config.block_on_detection {
        return Err(IbmHookError::Violation {
            message: detector_server_message(config, &per_message),
        });
    }
    Ok(())
}
