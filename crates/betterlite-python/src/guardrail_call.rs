mod context;
mod convert;
mod error;
mod functions;
mod invocation;
mod native;

use pyo3::prelude::*;

use error::NativeGuardrailError;
use functions::{
    add_guardrail_record, messages_for_call_type, sanitize_guardrail_response,
    summarize_guardrail_response, sync_log_target,
};

pub(crate) type RecordUpdate = (String, String);

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<native::NativeGuardrail>()?;
    module.add_class::<invocation::NativeGuardrailInvocation>()?;
    module.add(
        "NativeGuardrailError",
        module.py().get_type::<NativeGuardrailError>(),
    )?;
    module.add_function(wrap_pyfunction!(sanitize_guardrail_response, module)?)?;
    module.add_function(wrap_pyfunction!(add_guardrail_record, module)?)?;
    module.add_function(wrap_pyfunction!(sync_log_target, module)?)?;
    module.add_function(wrap_pyfunction!(summarize_guardrail_response, module)?)?;
    module.add_function(wrap_pyfunction!(messages_for_call_type, module)?)?;
    Ok(())
}
