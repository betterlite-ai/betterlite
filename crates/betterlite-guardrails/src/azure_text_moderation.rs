use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::azure_content_safety::api_version::resolve_api_version;
use crate::azure_content_safety::base::{AzureGuardrailBase, DEFAULT_TIMEOUT};
use crate::azure_content_safety::legacy_wire::TextCategory;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

pub mod config;
pub mod error;
pub mod hooks;
pub mod submission;
pub mod thresholds;
pub mod wire;

use config::{DEFAULT_CATEGORIES, OptionalParams};
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

impl AzureTextModerationGuardrail {
    pub fn new(params: AzureTextModerationParams, client: reqwest::Client) -> Self {
        let optional_params = params.optional_params;
        Self {
            base: AzureGuardrailBase {
                async_handler: client,
                api_key: params.api_key,
                api_base: params.api_base,
                api_version: resolve_api_version(params.api_version.as_deref()).to_owned(),
                timeout: params.timeout.unwrap_or(DEFAULT_TIMEOUT),
            },
            severity_threshold: optional_params.severity_threshold,
            severity_threshold_by_category: optional_params.severity_threshold_by_category,
            optional_params_request_body: BodyOptionalParams {
                categories: optional_params
                    .categories
                    .unwrap_or_else(|| DEFAULT_CATEGORIES.to_vec()),
                blocklist_names: optional_params.blocklist_names,
                halt_on_blocklist_hit: optional_params.halt_on_blocklist_hit.unwrap_or_default(),
                output_type: optional_params.output_type.unwrap_or_default(),
            },
        }
    }
}

impl GuardrailContract for AzureTextModerationGuardrail {
    fn apply_guardrail(
        &self,
        mut inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        async move {
            let Some(texts) = inputs.texts.take() else {
                return Ok(inputs);
            };
            for text in texts.iter().filter(|text| !text.is_empty()) {
                if let Err(error) = self.async_make_request(text).await {
                    return Err(guardrail_failure(error));
                }
            }
            inputs.texts = Some(texts);
            Ok(inputs)
        }
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        // LiteLLM's classmethod lists pre_call and post_call, in that order
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

fn guardrail_failure(error: AzureTextModerationError) -> GuardrailFailure {
    let status = error.status_code().as_u16();
    if matches!(error, AzureTextModerationError::Violation { .. }) {
        return GuardrailFailure::HttpObject {
            status,
            detail: IndexMap::from([("error".to_owned(), GuardrailValue::Text(error.to_string()))]),
        };
    }
    GuardrailFailure::Http {
        status,
        detail: error.to_string(),
    }
}

mod tests;
