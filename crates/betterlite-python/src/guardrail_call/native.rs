use std::collections::BTreeMap;

use pyo3::prelude::*;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::advisory::inject_advisory_message;
use betterlite_guardrails::guardrail_selection::gate::GuardrailSelection as _;

use super::convert;
use super::convert::settings::SettingsJson;
use super::error::CallError;

mod tests;

#[pyclass]
pub(crate) struct NativeGuardrail {
    base: GuardrailBase,
}

#[pymethods]
impl NativeGuardrail {
    #[new]
    pub(crate) fn new(settings_json: &str, strict_modes: bool) -> PyResult<Self> {
        let settings = convert::settings_from_text(settings_json)?;
        let base = GuardrailBase::new(settings, strict_modes).map_err(CallError::from)?;
        Ok(Self { base })
    }

    fn mode(&self) -> PyResult<String> {
        Ok(
            serde_json::to_string(&self.base.mode)
                .map_err(|source| CallError::Encode { source })?,
        )
    }

    fn update_params(&mut self, litellm_params: &str) -> PyResult<()> {
        let params: LitellmParams = convert::read("guardrail params", litellm_params)?;
        self.base.update_params(&params);
        Ok(())
    }

    fn settings(&self) -> PyResult<String> {
        Ok(
            serde_json::to_string(&SettingsJson::from(&self.base.settings))
                .map_err(|source| CallError::Encode { source })?,
        )
    }

    fn render_violation_message(&self, default: &str, context: &str) -> PyResult<String> {
        let entries = convert::text_context_from_text(context)?;
        let borrowed: Vec<(&str, &str)> = entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        Ok(self.base.render_violation_message(default, &borrowed))
    }

    #[staticmethod]
    fn inject_advisory_message(data: &str, message: &str) -> PyResult<(bool, String)> {
        let raw: BTreeMap<String, GuardrailValue> = convert::read("request data", data)?;
        let mut body = convert::exact_data(raw)?;
        let written = inject_advisory_message(&mut body, message);
        let text = serde_json::to_string(&body).map_err(|source| CallError::Encode { source })?;
        Ok((written, text))
    }

    fn should_run_guardrail(
        &self,
        data: &str,
        event_type: &str,
        suppressed: &str,
    ) -> PyResult<bool> {
        let raw: BTreeMap<String, GuardrailValue> = convert::read("request data", data)?;
        let body = convert::exact_data(raw)?;
        let hook = convert::event_type_from_name(event_type)?;
        let blocked = convert::suppressed_from_text(suppressed)?;
        let answer = self
            .base
            .should_run_guardrail(&body, hook, &blocked)
            .map_err(|source| CallError::Selection { source })?;
        Ok(answer)
    }

    fn event_hook_is_event_type(&self, event_type: &str) -> PyResult<bool> {
        Ok(self
            .base
            .event_hook_is_event_type(convert::event_type_from_name(event_type)?))
    }

    fn dynamic_request_body_params(&self, data: &str) -> PyResult<String> {
        let raw: BTreeMap<String, GuardrailValue> = convert::read("request data", data)?;
        let body = convert::exact_data(raw)?;
        let params = self
            .base
            .dynamic_request_body_params(&body)
            .map_err(|source| CallError::Selection { source })?;
        Ok(serde_json::to_string(&params).map_err(|source| CallError::Encode { source })?)
    }
}
