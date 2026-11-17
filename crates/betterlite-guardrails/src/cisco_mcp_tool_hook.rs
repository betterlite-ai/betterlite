use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::cisco_verdict_contracts::InspectionType;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::mcp_tool_hook::types::{McpPostCallResponse, McpToolHookCall};

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::hooks::record_applied_guardrail;
use crate::cisco_inspection::mcp::inspect_mcp_response;
use crate::cisco_mcp::context::hydrate_tool_context;
use crate::cisco_mcp_tool_hook::replace::{block, rewritten_items};
use crate::cisco_mcp_tool_hook::request::{
    guardrail_data, original_present, request_fields, scanned_response,
};
use crate::cisco_stream_scan::api_error_detail;
use crate::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use crate::cisco_verdict_enforcement::finalize::FinalizeOutcome;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_selection::gate::GuardrailSelection as _;

mod replace;
mod request;
mod tests;

pub struct CiscoMcpToolHook {
    pub guardrail: Arc<CiscoInspectionGuardrail>,
    pub selection: GuardrailBase,
}

impl CiscoMcpToolHook {
    fn gate(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        event: GuardrailEventHooks,
    ) -> Result<bool, LoggerError> {
        let callback = self.guardrail.settings.guardrail_name.clone();
        self.selection
            .should_run_guardrail(data, event, &BTreeSet::new())
            .map_err(|error| LoggerError::Delivery {
                callback,
                message: error.to_string(),
            })
    }

    fn runs(&self, data: &BTreeMap<String, GuardrailValue>) -> Result<bool, LoggerError> {
        if self.gate(data, GuardrailEventHooks::DuringMcpCall)? {
            return Ok(true);
        }
        self.gate(data, GuardrailEventHooks::PreMcpCall)
    }

    pub async fn inspect_tool_output(
        &self,
        request_data: &mut Fields,
        mut call: McpToolHookCall<'_>,
    ) -> Result<Option<McpPostCallResponse>, LoggerError> {
        let name = self.guardrail.settings.guardrail_name.as_str();
        if !self.runs(&guardrail_data(request_data, name)?)? {
            tracing::debug!(
                "Cisco AI Defense guardrail ({name}): no MCP mode configured \u{2014} skipping MCP response scan."
            );
            return Ok(None);
        }
        let Some(scanned) = scanned_response(call.response) else {
            tracing::debug!(
                "Cisco AI Defense guardrail: no MCP tool response payload to scan, skipping"
            );
            return Ok(None);
        };
        let inspected = inspect_mcp_response(&self.guardrail, request_data, &scanned).await;
        let Some(outcome) = inspected else {
            record_applied_guardrail(request_data, name);
            return Ok(None);
        };
        Ok(apply_outcome(outcome, &mut call, request_data, name))
    }
}

fn apply_outcome(
    outcome: FinalizeOutcome,
    call: &mut McpToolHookCall<'_>,
    request_data: &mut Fields,
    guardrail_name: &str,
) -> Option<McpPostCallResponse> {
    match outcome {
        FinalizeOutcome::Blocked(payload) => {
            let detail = serde_json::to_string(&payload)
                .map_or_else(|_| Json::Null, |text| Json::parse(&text));
            Some(block(call, request_data, &detail, guardrail_name))
        }
        FinalizeOutcome::ApiError(ApiErrorOutcome::Blocked(detail)) => {
            let error_detail = api_error_detail(&detail);
            Some(block(call, request_data, &error_detail, guardrail_name))
        }
        FinalizeOutcome::Redacted(Some(update)) => {
            let items = rewritten_items(&update);
            if original_present(call.kwargs) {
                call.original.content = items;
            } else {
                call.response.mcp_tool_call_response = items;
            }
            record_applied_guardrail(request_data, guardrail_name);
            None
        }
        FinalizeOutcome::Allow
        | FinalizeOutcome::Redacted(None)
        | FinalizeOutcome::Monitored
        | FinalizeOutcome::ApiError(ApiErrorOutcome::Unscanned) => {
            record_applied_guardrail(request_data, guardrail_name);
            None
        }
    }
}

#[async_trait]
impl CallbackLogger for CiscoMcpToolHook {
    fn callback_name(&self) -> &str {
        self.guardrail.settings.guardrail_name.as_str()
    }

    async fn async_post_mcp_tool_call_hook(
        &self,
        call: McpToolHookCall<'_>,
    ) -> Result<Option<McpPostCallResponse>, LoggerError> {
        if self.guardrail.settings.inspection_type != InspectionType::Mcp {
            return Ok(None);
        }
        let mut request_data = request_fields(call.kwargs);
        hydrate_tool_context(&mut request_data);
        self.inspect_tool_output(&mut request_data, call).await
    }
}
