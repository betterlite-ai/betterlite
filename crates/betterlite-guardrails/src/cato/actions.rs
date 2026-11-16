use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::DEFAULT_BLOCK_MESSAGE;
use super::error::CatoError;
use super::redaction::anonymize_request;
use super::wire::{AnalyzeResponse, RequiredAction};

pub(super) fn handle_actions(
    response: &AnalyzeResponse,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<(), CatoError> {
    let Some(required_action) = response.required_action.as_ref() else {
        tracing::debug!("Cato: no required action");
        return Ok(());
    };
    let Some(action_type) = required_action.action_type.as_deref() else {
        tracing::debug!("Cato: no required action specified");
        return Ok(());
    };
    match action_type {
        "monitor_action" => {
            tracing::info!("Cato: monitor action");
            Ok(())
        }
        "block_action" => {
            tracing::info!(policies = ?policy_names(response), "Cato: violation detected");
            Err(block_error(required_action))
        }
        "anonymize_action" => {
            tracing::info!("Cato: anonymize action");
            anonymize_request(response, data)
        }
        other => {
            tracing::error!(action_type = other, "Cato: unsupported action");
            Ok(())
        }
    }
}

pub(super) fn policy_names(response: &AnalyzeResponse) -> Vec<String> {
    response
        .analysis_result
        .as_ref()
        .and_then(|result| result.policy_drill_down.as_ref())
        .map(|policies| policies.keys().cloned().collect())
        .unwrap_or_default()
}

// LiteLLM raises HTTPException(400, detection_message); an absent message takes betterlite's own
pub(super) fn block_error(required_action: &RequiredAction) -> CatoError {
    let message = required_action
        .detection_message
        .clone()
        .unwrap_or_else(|| DEFAULT_BLOCK_MESSAGE.to_owned());
    CatoError::Blocked { message }
}
