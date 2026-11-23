use super::convert::{self, McpErrorKind, NativeMcpErrorPayload};
use betterlite_mcp::tools::schemas::{
    McpTool, mcp_input_schema_normalized, mcp_tool_to_anthropic, mcp_tool_to_openai,
    mcp_tool_to_responses_api,
};
use betterlite_mcp::tools::tool_calls::{OpenAiToolCall, openai_tool_call_to_mcp};
use pyo3::prelude::*;
// litellm.experimental_mcp_client.tools.transform_mcp_tool_to_openai_tool
#[pyfunction]
pub(crate) fn tool_to_openai(tool: &str) -> PyResult<String> {
    let parsed: McpTool = convert::parse(tool)?;
    let converted = mcp_tool_to_openai(&parsed);
    convert::encode(&converted).map_err(PyErr::from)
}
// transform_mcp_tool_to_openai_responses_api_tool
#[pyfunction]
pub(crate) fn tool_to_responses(tool: &str) -> PyResult<String> {
    let parsed: McpTool = convert::parse(tool)?;
    let converted = mcp_tool_to_responses_api(&parsed);
    convert::encode(&converted).map_err(PyErr::from)
}
// transform_mcp_tool_to_anthropic_tool refuses a schema the Anthropic API cannot take
#[pyfunction]
pub(crate) fn tool_to_anthropic(tool: &str) -> PyResult<String> {
    let parsed: McpTool = convert::parse(tool)?;
    let converted = mcp_tool_to_anthropic(&parsed).map_err(|error| {
        NativeMcpErrorPayload::plain(McpErrorKind::ValueError, error.to_string())
    })?;
    convert::encode(&converted).map_err(PyErr::from)
}
// _normalize_mcp_input_schema fills in type and properties; a null schema normalizes like an empty one
#[pyfunction]
pub(crate) fn normalize_input_schema(schema: Option<&str>) -> PyResult<String> {
    let parsed = schema.map(convert::parse).transpose()?;
    let converted = mcp_input_schema_normalized(parsed.as_ref());
    convert::encode(&converted).map_err(PyErr::from)
}
// transform_openai_tool_call_request_to_mcp_tool_call_request
#[pyfunction]
pub(crate) fn tool_call_to_mcp(tool_call: &str) -> PyResult<String> {
    let call: OpenAiToolCall = convert::parse(tool_call)?;
    let converted = openai_tool_call_to_mcp(&call);
    convert::encode(&converted).map_err(PyErr::from)
}
