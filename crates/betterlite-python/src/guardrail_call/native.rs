use pyo3::prelude::*;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::advisory::inject_advisory_message;
use betterlite_guardrails::guardrail_selection::gate::GuardrailSelection;

use super::convert;
use super::convert::settings::SettingsJson;
use super::error::GuardrailCallError;

#[pyclass]
pub(crate) struct NativeGuardrail {
    base: GuardrailBase,
}

#[pymethods]
impl NativeGuardrail {
    #[new]
    pub(crate) fn new(settings_json: &str, strict_modes: bool) -> PyResult<Self> {
        let settings = convert::settings_from_text(settings_json)?;
        GuardrailBase::new(settings, strict_modes)
            .map(|base| Self { base })
            .map_err(GuardrailCallError::from)
    }

    fn mode(&self) -> PyResult<String> {
        let mode = self
            .base
            .mode
            .as_ref()
            .map(convert::settings::mode_to_value);
        let value = mode.unwrap_or(GuardrailValue::Null);
        convert::value_to_text(&value)
    }

    fn update_params(&mut self, litellm_params: &str) -> PyResult<()> {
        let params = convert::params_from_text(litellm_params)?;
        self.base.update_params(&params);
        Ok(())
    }

    fn settings(&self) -> PyResult<String> {
        let json = SettingsJson::from(&self.base.settings);
        serde_json::to_string(&json).map_err(|source| GuardrailCallError::Json {
            field: "guardrail settings",
            source,
        })
    }

    fn render_violation_message(&self, default: &str, context: &str) -> PyResult<String> {
        let entries = convert::text_context_from_text(context)?;
        let borrowed: Vec<(&str, &str)> = entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        Ok(self.base.render_violation_message(default, &borrowed))
    }

    fn inject_advisory_message(&self, data: &str, message: &str) -> PyResult<(bool, String)> {
        let mut body = convert::data_from_text(data)?;
        let written = inject_advisory_message(&mut body, message);
        let text = convert::data_to_text(&body)?;
        Ok((written, text))
    }

    fn should_run_guardrail(
        &self,
        data: &str,
        event_type: &str,
        suppressed: &str,
    ) -> PyResult<bool> {
        let body = convert::data_from_text(data)?;
        let hook = convert::event_type_from_name(event_type)?;
        let suppressed = convert::suppressed_from_text(suppressed)?;
        let answer = self
            .base
            .should_run_guardrail(&body, hook, &suppressed)
            .map_err(|source| GuardrailCallError::Selection { source })?;
        Ok(answer)
    }

    fn event_hook_is_event_type(&self, event_type: &str) -> PyResult<bool> {
        let hook = convert::event_type_from_name(event_type)?;
        let answer = self.base.event_hook_is_event_type(hook);
        Ok(answer)
    }

    fn dynamic_request_body_params(&self, data: &str) -> PyResult<String> {
        let body = convert::data_from_text(data)?;
        let params = self
            .base
            .dynamic_request_body_params(&body)
            .map_err(|source| GuardrailCallError::Selection { source })?;
        convert::data_to_text(&params)
    }
}
