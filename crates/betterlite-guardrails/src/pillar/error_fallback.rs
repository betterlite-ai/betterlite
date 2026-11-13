use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::error::PillarError;
use super::settings::{FallbackAction, PillarSettings};
use crate::guardrail_base::failure::GuardrailFailure;

const UNAVAILABLE_STATUS: u16 = 503;
const UNAVAILABLE_ERROR: &str = "Pillar Security Guardrail Unavailable";
const UNAVAILABLE_MESSAGE: &str =
    "Security scanning service is temporarily unavailable and fallback is set to block";

pub fn handle_api_error(
    error: &PillarError,
    data: BTreeMap<String, GuardrailValue>,
    settings: &PillarSettings,
) -> Result<BTreeMap<String, GuardrailValue>, PillarError> {
    match settings.fallback_on_error {
        FallbackAction::Allow => {
            tracing::warn!(
                "Pillar Guardrail: API unavailable, proceeding without scanning (fallback_on_error=allow)"
            );
            Ok(data)
        }
        FallbackAction::Block => {
            tracing::warn!(
                "Pillar Guardrail: API unavailable, blocking request (fallback_on_error=block)"
            );
            let detail = IndexMap::from([
                (
                    "error".to_owned(),
                    GuardrailValue::Text(UNAVAILABLE_ERROR.to_owned()),
                ),
                (
                    "message".to_owned(),
                    GuardrailValue::Text(UNAVAILABLE_MESSAGE.to_owned()),
                ),
                (
                    "original_error".to_owned(),
                    GuardrailValue::Text(error.to_string()),
                ),
            ]);
            Err(PillarError::Http(GuardrailFailure::HttpObject {
                status: UNAVAILABLE_STATUS,
                detail,
            }))
        }
    }
}
