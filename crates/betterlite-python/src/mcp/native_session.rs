use std::sync::Arc;

use betterlite_mcp::tools::tool_calls::OpenAiToolCall;
use pyo3::prelude::*;

use super::awaited;
use super::convert::{self, CallToolWire};
use super::session::{Command, OpenSession};

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeMcpSession {
    pub(crate) open: Arc<OpenSession>,
}

#[pymethods]
impl NativeMcpSession {
    #[pyo3(signature = (cursor = None))]
    fn list_tools_page<'py>(
        &self,
        py: Python<'py>,
        cursor: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let open = Arc::clone(&self.open);
        awaited(
            py,
            async move { open.exchange(Command::ListPage(cursor)).await },
        )
    }

    fn call_tool<'py>(&self, py: Python<'py>, request: &str) -> PyResult<Bound<'py, PyAny>> {
        let wire: CallToolWire = convert::parse(request)?;
        let command = Command::CallTool(convert::call_request(&wire), wire.allow_input_required);
        let open = Arc::clone(&self.open);
        awaited(py, async move { open.exchange(command).await })
    }

    #[pyo3(signature = (tool_format, listing_deadline = None))]
    fn load_tools<'py>(
        &self,
        py: Python<'py>,
        tool_format: &str,
        listing_deadline: Option<f64>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let format = convert::tool_format(tool_format)?;
        let deadline = convert::deadline(listing_deadline)?;
        let command = Command::LoadTools(format, deadline);
        let open = Arc::clone(&self.open);
        awaited(py, async move { open.exchange(command).await })
    }

    fn call_openai_tool<'py>(
        &self,
        py: Python<'py>,
        tool_call: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call: OpenAiToolCall = convert::parse(tool_call)?;
        let open = Arc::clone(&self.open);
        awaited(py, async move {
            open.exchange(Command::CallOpenAiTool(call)).await
        })
    }

    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let open = Arc::clone(&self.open);
        awaited(py, async move {
            open.close().await;
            Ok(())
        })
    }
}
