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

struct InspectDispatch<'ctx> {
    guardrail: &'ctx CiscoInspectionGuardrail,
    scan: ScanContext,
    request_data: &'ctx mut Fields,
    response: ResponseSurface<'ctx>,
    skip_message: &'static str,
}

impl<'ctx> InspectDispatch<'ctx> {
    const fn for_request(
        guardrail: &'ctx CiscoInspectionGuardrail,
        request_data: &'ctx mut Fields,
    ) -> Self {
        let scan = ScanContext {
            surface: InspectionType::Mcp,
            direction: Direction::Input,
        };
        let skip_message =
            "Cisco AI Defense guardrail: could not build MCP request payload, skipping";
        Self {
            guardrail,
            scan,
            request_data,
            response: ResponseSurface::None,
            skip_message,
        }
    }

    const fn for_response(
        guardrail: &'ctx CiscoInspectionGuardrail,
        request_data: &'ctx mut Fields,
        response: &'ctx Json,
    ) -> Self {
        Self {
            guardrail,
            scan: ScanContext {
                surface: InspectionType::Mcp,
                direction: Direction::Output,
            },
            request_data,
            response: ResponseSurface::McpResponse(response),
            skip_message: "Cisco AI Defense guardrail: could not build MCP response payload, skipping",
        }
    }
}

// Mirrors LiteLLM's fn _inspect_mcp_request.
pub async fn inspect_mcp_request(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
) -> Option<FinalizeOutcome> {
    let payload = build_request_payload(request_data);
    dispatch_if_built(
        InspectDispatch::for_request(guardrail, request_data),
        payload,
    )
    .await
}

// Mirrors LiteLLM's fn _inspect_mcp_response: same None-signals-skip contract as the request side.
pub async fn inspect_mcp_response(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    response: &Json,
) -> Option<FinalizeOutcome> {
    let payload = build_response_payload(request_data, response);
    let call = InspectDispatch::for_response(guardrail, request_data, response);
    dispatch_if_built(call, payload).await
}

async fn dispatch_if_built<Payload>(
    call: InspectDispatch<'_>,
    payload: Option<Payload>,
) -> Option<FinalizeOutcome>
where
    Payload: serde::Serialize + Sync,
{
    let Some(built) = payload else {
        tracing::debug!("{}", call.skip_message);
        return None;
    };
    Some(
        dispatch(
            call.guardrail,
            call.scan,
            &built,
            call.request_data,
            call.response,
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
