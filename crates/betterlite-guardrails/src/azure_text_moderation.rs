use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::azure_content_safety::base::AzureGuardrailBase;
use crate::azure_content_safety::legacy_wire::TextCategory;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

pub mod config;
pub mod error;
pub mod guardrail;
pub mod wire;

use config::OptionalParams;
use error::AzureTextModerationError;
use wire::BodyOptionalParams;

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];

#[derive(Debug, Clone)]
pub struct AzureTextModerationGuardrail {
    pub base: AzureGuardrailBase,
    pub severity_threshold: Option<i64>,
    pub severity_threshold_by_category: Option<BTreeMap<TextCategory, i64>>,
    pub optional_params_request_body: BodyOptionalParams,
}

#[derive(Debug, Clone)]
pub struct AzureTextModerationParams {
    pub api_key: SecretString,
    pub api_base: String,
    pub api_version: Option<String>,
    pub timeout: Option<Duration>,
    pub optional_params: OptionalParams,
}

impl GuardrailContract for AzureTextModerationGuardrail {
    async fn apply_guardrail(
        &self,
        mut inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let Some(texts) = inputs.texts.take() else {
            return Ok(inputs);
        };
        if let Err(error) = self.scan_texts(&texts).await {
            return Err(guardrail_failure(error));
        }
        inputs.texts = Some(texts);
        Ok(inputs)
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        // LiteLLM's classmethod lists pre_call and post_call, in that order
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

fn guardrail_failure(error: AzureTextModerationError) -> GuardrailFailure {
    let status = error.status_code().as_u16();
    match error {
        violation @ AzureTextModerationError::Violation { .. } => GuardrailFailure::HttpObject {
            status,
            detail: IndexMap::from([(
                "error".to_owned(),
                GuardrailValue::Text(violation.to_string()),
            )]),
        },
        other @ (AzureTextModerationError::ContentSafety(_)
        | AzureTextModerationError::NoChunks
        | AzureTextModerationError::Frame { .. }) => GuardrailFailure::Http {
            status,
            detail: other.to_string(),
        },
    }
}

mod tests;
