use std::future::Future;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

mod capture;
mod client;
mod convert;
mod native_client;
mod native_session;
mod session;
mod setup;
mod transforms;

use self::convert::{ConvertError, NativeMcpErrorPayload};
use self::native_client::NativeMcpClient;
use self::native_session::NativeMcpSession;
use self::transforms::{
    normalize_input_schema, tool_call_to_mcp, tool_to_anthropic, tool_to_openai, tool_to_responses,
};

// The mcp family: the client, the session handle, the error every failure carries and five pure transforms
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeMcpClient>()?;
    module.add_class::<NativeMcpSession>()?;
    module.add("NativeMcpError", module.py().get_type::<NativeMcpError>())?;
    module.add_function(wrap_pyfunction!(tool_to_openai, module)?)?;
    module.add_function(wrap_pyfunction!(tool_to_responses, module)?)?;
    module.add_function(wrap_pyfunction!(tool_to_anthropic, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_input_schema, module)?)?;
    module.add_function(wrap_pyfunction!(tool_call_to_mcp, module)?)?;
    Ok(())
}

// Every native method hands Python one shape: a future of JSON text, or its mapped exception
pub(crate) fn awaited<'py, Value>(
    py: Python<'py>,
    work: impl Future<Output = Result<Value, NativeMcpErrorPayload>> + Send + 'static,
) -> PyResult<Bound<'py, PyAny>>
where
    Value: for<'value> IntoPyObject<'value> + Send + 'static,
{
    future_into_py(py, async move { work.await.map_err(PyErr::from) })
}

// litellm's MCP flows catch its own error class, so every failure arrives as this exception carrying JSON text
create_exception!(betterlite_native, NativeMcpError, PyException);

impl From<NativeMcpErrorPayload> for PyErr {
    fn from(payload: NativeMcpErrorPayload) -> Self {
        match serde_json::to_string(&payload) {
            Ok(text) => NativeMcpError::new_err(text),
            Err(_encoding) => PyRuntimeError::new_err(payload.message),
        }
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        Self::from(NativeMcpErrorPayload::from(error))
    }
}
