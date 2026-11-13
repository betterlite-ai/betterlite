use std::time::SystemTime;

use pyo3::prelude::*;

use betterlite_guardrails::guardrail_records::invocation::GuardrailInvocation;

use super::RecordUpdate;
use super::context::{RecordStep, record_call};
use super::convert;
use super::error::CallError;

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
        .map_err(CallError::from)?;
        Py::new(
            py,
            Self {
                invocation: Some(invocation),
            },
        )
    }

    fn record_own(&mut self, data: &str, input: &str, context: &str) -> PyResult<RecordUpdate> {
        let record = convert::record_input::record_input_from_text(input)?;
        let step = RecordStep::Own(record);
        Ok(record_call(&mut self.invocation, step, data, context)?)
    }

    fn finish_success(
        &mut self,
        data: &str,
        response: Option<&str>,
        context: &str,
    ) -> PyResult<RecordUpdate> {
        let present = convert::response_value(response)?;
        let step = RecordStep::Finish(present);
        Ok(record_call(&mut self.invocation, step, data, context)?)
    }
}
