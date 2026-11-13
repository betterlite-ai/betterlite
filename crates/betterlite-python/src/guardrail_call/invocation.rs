use std::time::SystemTime;

use pyo3::prelude::*;

use betterlite_guardrails::guardrail_records::invocation::GuardrailInvocation;

use super::context::record_parts;
use super::convert;
use super::error::GuardrailCallError;

#[pyclass]
pub(crate) struct NativeGuardrailInvocation {
    invocation: Option<GuardrailInvocation>,
}

#[pymethods]
impl NativeGuardrailInvocation {
    #[staticmethod]
    fn begin(
        py: Python<'_>,
        hook: &str,
        request_data: &str,
        inputs: Option<&str>,
    ) -> PyResult<Py<Self>> {
        let selected_hook = convert::hook_from_name(hook)?;
        let data = convert::data_from_text(request_data)?;
        let api_inputs = convert::inputs_from_text(inputs)?;
        let invocation = GuardrailInvocation::begin(
            selected_hook,
            &data,
            api_inputs.as_ref(),
            SystemTime::now(),
        )
        .map_err(GuardrailCallError::from)?;
        Py::new(
            py,
            Self {
                invocation: Some(invocation),
            },
        )
    }

    fn record_own(
        &mut self,
        data: &str,
        record_input: &str,
        context: &str,
    ) -> PyResult<(String, String)> {
        let mut body = convert::data_from_text(data)?;
        let input = convert::record_input::record_input_from_text(record_input)?;
        let mut parts = record_parts(context)?;
        let invocation = self
            .invocation
            .as_mut()
            .ok_or(GuardrailCallError::Finished)?;
        let mut record_context = parts.context(&mut body);
        invocation
            .record_own(&mut record_context, input)
            .map_err(GuardrailCallError::from)?;
        let text = convert::data_to_text(&body)?;
        Ok((text, parts.log_target_text()?))
    }

    fn finish_success(
        &mut self,
        data: &str,
        response: Option<&str>,
        context: &str,
    ) -> PyResult<(String, String)> {
        let mut body = convert::data_from_text(data)?;
        let present = convert::optional_value_from_text(response)?;
        let mut parts = record_parts(context)?;
        let invocation = self.invocation.take().ok_or(GuardrailCallError::Finished)?;
        let mut record_context = parts.context(&mut body);
        invocation
            .finish_success(&mut record_context, present.as_ref(), SystemTime::now())
            .map_err(GuardrailCallError::from)?;
        let text = convert::data_to_text(&body)?;
        Ok((text, parts.log_target_text()?))
    }
}
