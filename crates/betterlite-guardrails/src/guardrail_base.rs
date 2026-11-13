pub mod advisory;
pub mod contract;
pub mod error;
pub mod failure;
pub mod hooks;
pub mod inputs;
pub mod mask;
pub mod session;
pub mod settings;
mod tests;

use std::collections::BTreeMap;

use strfmt::{FmtError, strfmt_map};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::{OnSensitiveData, OnViolation};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use error::GuardrailBaseError;
use failure::{GuardrailFailure, ModifyResponseFailure};
use hooks::validate_event_hook;
use settings::GuardrailSettings;

#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailBase {
    pub settings: GuardrailSettings,
    pub mode: Option<LitellmParamsMode>,
}

impl GuardrailBase {
    pub fn new(
        settings: GuardrailSettings,
        strict_modes: bool,
    ) -> Result<Self, GuardrailBaseError> {
        let supported = settings
            .supported_event_hooks
            .as_deref()
            .unwrap_or_default();
        if !supported.is_empty()
            && let Err(error) =
                validate_event_hook(settings.event_hook.as_ref(), supported, &settings.switches)
        {
            if strict_modes {
                return Err(error);
            }
            tracing::warn!(
                "{error}. LITELLM_STRICT_GUARDRAIL_MODES=false; continuing with unsupported event_hook. Set the env var to true (default) to enforce validation and fail at startup."
            );
        }
        Ok(Self {
            settings,
            mode: None,
        })
    }

    // LiteLLM's update_in_memory_litellm_params: the params' defaults overwrite too
    pub fn update_params(&mut self, params: &LitellmParams) {
        let fields = &params.base.fields;
        let settings = &mut self.settings;
        settings.default_on = fields.default_on.unwrap_or_default();
        settings.mask_request_content = fields.mask_request_content.unwrap_or_default();
        settings.mask_response_content = fields.mask_response_content.unwrap_or_default();
        settings
            .violation_message_template
            .clone_from(&fields.violation_message_template);
        settings.end_session_after_n_fails = fields.end_session_after_n_fails;
        settings.on_violation = fields.on_violation.map(|action| match action {
            OnViolation::Warn => "warn".to_owned(),
            OnViolation::EndSession => "end_session".to_owned(),
            OnViolation::Block => "block".to_owned(),
            OnViolation::Alert => "alert".to_owned(),
        });
        settings
            .realtime_violation_message
            .clone_from(&fields.realtime_violation_message);
        settings.on_sensitive_data = fields.on_sensitive_data.map(|action| match action {
            OnSensitiveData::Block => "block".to_owned(),
            OnSensitiveData::Route => "route".to_owned(),
        });
        settings
            .sensitive_data_route_to_model
            .clone_from(&fields.sensitive_data_route_to_model);
        settings.sticky_session_routing = params.base.handling_flags.sticky_session_routing;
        settings.run_in_parallel = fields.run_in_parallel.unwrap_or_default();
        settings.scan_raw_request = fields.scan_raw_request.unwrap_or_default();
        settings.only_scan_new_messages = params.base.scan_flags.only_scan_new_messages;
        settings.timeout = fields.timeout;
        self.mode = Some(params.mode.clone());
    }

    #[must_use]
    pub fn render_violation_message(&self, default: &str, context: &[(&str, &str)]) -> String {
        let Some(template) = self
            .settings
            .violation_message_template
            .as_deref()
            .filter(|template| !template.is_empty())
        else {
            return default.to_owned();
        };
        let rendered = strfmt_map(template, |mut field| {
            // a context entry overrides default_message, and a later duplicate overrides an earlier one
            let value = context
                .iter()
                .rev()
                .find(|(name, _value)| *name == field.key)
                .map(|(_name, value)| *value)
                .or_else(|| (field.key == "default_message").then_some(default))
                .ok_or_else(|| FmtError::KeyError(field.key.to_owned()))?;
            field.str(value)
        });
        rendered.unwrap_or_else(|error| {
            let name = self.settings.guardrail_name.as_deref().unwrap_or("None");
            tracing::warn!(
                "Failed to format violation message template for guardrail {name}: {error}"
            );
            default.to_owned()
        })
    }

    // LiteLLM raises here, so the caller returns this value as its Err
    #[must_use]
    pub fn passthrough_failure(
        &self,
        message: &str,
        request_data: &BTreeMap<String, GuardrailValue>,
        detection_info: Option<BTreeMap<String, GuardrailValue>>,
        original_response: Option<GuardrailValue>,
    ) -> GuardrailFailure {
        let model = match request_data.get("model") {
            Some(GuardrailValue::Text(model)) => model.clone(),
            _ => "unknown".to_owned(),
        };
        GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
            message: message.to_owned(),
            model,
            request_data: request_data.clone(),
            guardrail_name: self.settings.guardrail_name.clone(),
            detection_info: detection_info.unwrap_or_default(),
            original_response,
        }))
    }
}
