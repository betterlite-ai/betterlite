use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_selection::gate::GuardrailSelection as _;

use super::arguments::{ArgumentFilter, filter_argument_value};
use super::error::ArgumentFilterError;

pub const MCP_CALL_LABEL: &str = "MCP tool call";
pub const MCP_CALL_TYPE: &str = "call_mcp_tool";

pub fn scan_mcp_tool_call_arguments(
    base: &GuardrailBase,
    filter: &ArgumentFilter<'_>,
    call_type: Option<&str>,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<(), ArgumentFilterError> {
    if !base.event_hook_is_event_type(GuardrailEventHooks::PreMcpCall)
        || call_type.is_some_and(|value| value != MCP_CALL_TYPE)
    {
        return Ok(());
    }
    let Some(GuardrailValue::Text(tool_name)) = request_data.get("mcp_tool_name") else {
        return Ok(());
    };
    if tool_name.is_empty() {
        return Ok(());
    }
    let Some(GuardrailValue::Object(raw_arguments)) = request_data.get("mcp_arguments").cloned()
    else {
        return Ok(());
    };
    if raw_arguments.is_empty() {
        return Ok(());
    }
    let raw = GuardrailValue::Object(raw_arguments);
    let filtered = filter_argument_value(filter, &raw, detections, MCP_CALL_LABEL, 0)?;
    if filtered == raw {
        return Ok(());
    }
    request_data.insert("mcp_arguments".to_owned(), filtered.clone());
    request_data.insert("modified_arguments".to_owned(), filtered);
    Ok(())
}
