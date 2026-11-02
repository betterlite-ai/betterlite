use std::ops::Mul as _;
use std::time::Instant;

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_configuration::records::{Direction, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::transport::post_inspection;
use crate::cisco_mcp::envelope::{build_request_payload, build_response_payload};
use crate::cisco_verdict_enforcement::finalize::{
    FinalizeContext, FinalizeOutcome, ResponseSurface, finalize_inspection,
};
use crate::cisco_verdict_enforcement::handle_api_error;

const PROVIDER_NAME: &str = "cisco_ai_defense";

// Mirrors LiteLLM's fn _inspect_mcp_request.
pub async fn inspect_mcp_request(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
) -> Option<FinalizeOutcome> {
    let Some(payload) = build_request_payload(request_data) else {
        tracing::debug!(
            "Cisco AI Defense guardrail: could not build MCP request payload, skipping"
        );
        return None;
    };
    let scan = ScanContext {
        surface: InspectionType::Mcp,
        direction: Direction::Input,
    };
    Some(
        dispatch(
            guardrail,
            scan,
            &payload,
            request_data,
            ResponseSurface::None,
        )
        .await,
    )
}

// Mirrors LiteLLM's fn _inspect_mcp_response: same None-signals-skip contract as the request side.
pub async fn inspect_mcp_response(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    response: &Json,
) -> Option<FinalizeOutcome> {
    let Some(payload) = build_response_payload(request_data, response) else {
        tracing::debug!(
            "Cisco AI Defense guardrail: could not build MCP response payload, skipping"
        );
        return None;
    };
    let scan = ScanContext {
        surface: InspectionType::Mcp,
        direction: Direction::Output,
    };
    Some(
        dispatch(
            guardrail,
            scan,
            &payload,
            request_data,
            ResponseSurface::McpResponse(response),
        )
        .await,
    )
}

async fn dispatch<Payload>(
    guardrail: &CiscoInspectionGuardrail,
    scan: ScanContext,
    payload: &Payload,
    request_data: &mut Fields,
    response: ResponseSurface<'_>,
) -> FinalizeOutcome
where
    Payload: serde::Serialize + Sync,
{
    let url = format!(
        "{}{}",
        guardrail.settings.api_base, guardrail.settings.inspect_path
    );
    let start = Instant::now();
    match post_inspection(guardrail, &url, payload, "mcp").await {
        Ok(inspect_response) => {
            let duration_ms = start.elapsed().as_secs_f64().mul(1000.0_f64);
            let finalize = FinalizeContext {
                scan,
                duration_ms,
                provider_name: PROVIDER_NAME,
                guardrail_name: &guardrail.settings.guardrail_name,
                on_flagged_action: guardrail.settings.on_flagged_action,
                fallback_on_error: guardrail.settings.fallback_on_error,
            };
            finalize_inspection(&inspect_response, request_data, response, &finalize)
        }
        Err(error) => {
            tracing::debug!(error = %error, "Cisco AI Defense MCP inspection failed");
            FinalizeOutcome::ApiError(handle_api_error(
                scan.surface,
                "CiscoAIDefenseGuardrailAPIError",
                guardrail.settings.fallback_on_error,
            ))
        }
    }
}

mod tests;
