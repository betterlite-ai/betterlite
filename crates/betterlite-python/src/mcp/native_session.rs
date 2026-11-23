use std::sync::Arc;

use betterlite_mcp::tools::tool_calls::OpenAiToolCall;
use pyo3::prelude::*;

use super::convert::{self, CallToolWire, call_request};
use super::session::{Command, OpenSession};
use super::{Answer, awaited};

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeMcpSession {
    pub(crate) open: Arc<OpenSession>,
}

#[pymethods]
impl NativeMcpSession {
    #[pyo3(signature = (cursor = None))]
    fn list_tools_page<'py>(&self, py: Python<'py>, cursor: Option<String>) -> Answer<'py> {
        let open = Arc::clone(&self.open);
        let work = async move { open.exchange(Command::ListPage(cursor)).await };
        awaited(py, work)
    }

    fn call_tool<'py>(&self, py: Python<'py>, request: &str) -> Answer<'py> {
        let wire: CallToolWire = convert::parse(request)?;
        let command = Command::CallTool(call_request(&wire), wire.allow_input_required);
        let open = Arc::clone(&self.open);
        let work = async move { open.exchange(command).await };
        awaited(py, work)
    }

    #[pyo3(signature = (tool_format, listing_deadline = None))]
    fn load_tools<'py>(
        &self,
        py: Python<'py>,
        tool_format: &str,
        listing_deadline: Option<f64>,
    ) -> Answer<'py> {
        let format = convert::tool_format(tool_format)?;
        let deadline = convert::deadline(listing_deadline)?;
        let command = Command::LoadTools(format, deadline);
        let open = Arc::clone(&self.open);
        let work = async move { open.exchange(command).await };
        awaited(py, work)
    }

    fn call_openai_tool<'py>(&self, py: Python<'py>, tool_call: &str) -> Answer<'py> {
        let call: OpenAiToolCall = convert::parse(tool_call)?;
        let command = Command::CallOpenAiTool(call);
        let open = Arc::clone(&self.open);
        let work = async move { open.exchange(command).await };
        awaited(py, work)
    }

    fn aclose<'py>(&self, py: Python<'py>) -> Answer<'py> {
        let open = Arc::clone(&self.open);
        let work = async move {
            open.close().await;
            Ok(())
        };
        awaited(py, work)
    }
}
