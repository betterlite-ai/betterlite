use std::collections::BTreeMap;
use std::time::SystemTime;

use pyo3::prelude::*;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};

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
        // LiteLLM infers the event type from func.__name__ and gives every unknown name no event type and no baseline
        let selected_hook = convert::find(&convert::HOOK_NAMES, hook)
            .unwrap_or(GuardrailHookName::StreamingIterator);
        let raw: BTreeMap<String, GuardrailValue> = convert::read("request data", request_data)?;
        let data = convert::exact_data(raw)?;
        let api_inputs: Option<GuardrailApiInputs> =
            convert::read("guardrail inputs", inputs.unwrap_or("null"))?;
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
        let raw: Option<GuardrailValue> =
            convert::read("guardrail response", response.unwrap_or("null"))?;
        let step = RecordStep::Finish(raw.map(convert::exact_value).transpose()?);
        Ok(record_call(&mut self.invocation, step, data, context)?)
    }
}
