use betterlite_config::sections::cisco_verdict_contracts::{
    FallbackAction, FlaggedAction, InspectionType,
};

use crate::cisco_configuration::records::{CiscoVerdict, Direction, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::redact::TextUpdate;
use crate::cisco_redaction::chat_input::redact_chat_input;
use crate::cisco_redaction::chat_output::redact_chat_output;
use crate::cisco_redaction::mcp::{apply_redaction_mcp_input, apply_redaction_mcp_output};
use crate::cisco_verdict::{Action, extract_jsonrpc_error};
use crate::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use crate::cisco_verdict_enforcement::block_payload::BlockPayload;
use crate::cisco_verdict_enforcement::metadata::stash_verdict_on_request;
use crate::cisco_verdict_enforcement::verdict_resolution::resolve_verdict;
use crate::cisco_verdict_enforcement::{build_block_payload, handle_api_error, log_decision};

pub enum ResponseSurface<'surface> {
    None,
    ChatResponse(&'surface mut Fields),
    McpResponse(&'surface Json),
}

pub struct RedactionOutcome {
    pub applied: bool,
    pub mcp_output_update: Option<TextUpdate>,
}

// Bundles the caller-supplied, read-only inputs finalize_inspection needs beyond the mutable request_data and response surfaces.
pub struct FinalizeContext<'ctx> {
    pub scan: ScanContext,
    pub duration_ms: f64,
    pub provider_name: &'ctx str,
    pub guardrail_name: &'ctx str,
    pub on_flagged_action: FlaggedAction,
    pub fallback_on_error: FallbackAction,
}

pub enum FinalizeOutcome {
    Allow,
    Redacted(Option<TextUpdate>),
    Monitored,
    Blocked(BlockPayload),
    ApiError(ApiErrorOutcome),
}

// Mirrors LiteLLM's fn _finalize_inspection: a JSON-RPC error envelope becomes an API error, otherwise the verdict is resolved, stashed, logged and dispatched.
#[must_use]
pub fn finalize_inspection(
    inspect_response: &Json,
    request_data: &mut Fields,
    response: ResponseSurface<'_>,
    finalize: &FinalizeContext<'_>,
) -> FinalizeOutcome {
    if extract_jsonrpc_error(inspect_response).is_some() {
        return FinalizeOutcome::ApiError(handle_api_error(
            finalize.scan.surface,
            "CiscoAIDefenseGuardrailAPIError",
            finalize.fallback_on_error,
        ));
    }
    let verdict = resolve_verdict(inspect_response);
    stash_verdict_on_request(request_data, finalize.scan, &verdict);
    log_decision(
        finalize.scan,
        &verdict,
        finalize.duration_ms,
        finalize.guardrail_name,
        request_data,
    );
    match verdict.action.unwrap_or(Action::Allow) {
        Action::Allow => FinalizeOutcome::Allow,
        Action::Redact => decide_redact(request_data, response, finalize, &verdict),
        Action::Block => decide_flagged_action(finalize, &verdict),
    }
}

fn decide_redact(
    request_data: &mut Fields,
    response: ResponseSurface<'_>,
    finalize: &FinalizeContext<'_>,
    verdict: &CiscoVerdict,
) -> FinalizeOutcome {
    let outcome = match finalize.scan.direction {
        Direction::Input => apply_redaction_input(request_data, finalize.scan.surface, verdict),
        Direction::Output => apply_redaction_output(response, finalize.scan.surface, verdict),
    };
    if outcome.applied {
        FinalizeOutcome::Redacted(outcome.mcp_output_update)
    } else {
        decide_flagged_action(finalize, verdict)
    }
}

fn decide_flagged_action(
    finalize: &FinalizeContext<'_>,
    verdict: &CiscoVerdict,
) -> FinalizeOutcome {
    match finalize.on_flagged_action {
        FlaggedAction::Block => FinalizeOutcome::Blocked(build_block_payload(
            finalize.scan,
            verdict,
            finalize.provider_name,
            finalize.guardrail_name,
        )),
        FlaggedAction::Monitor => FinalizeOutcome::Monitored,
    }
}

const fn redaction_outcome(
    applied: bool,
    mcp_output_update: Option<TextUpdate>,
) -> RedactionOutcome {
    RedactionOutcome {
        applied,
        mcp_output_update,
    }
}

// Backs LiteLLM's fn _apply_redaction's input branch: dispatches to the surface-specific redaction helper.
pub(crate) fn apply_redaction_input(
    request_data: &mut Fields,
    surface: InspectionType,
    verdict: &CiscoVerdict,
) -> RedactionOutcome {
    if surface == InspectionType::Mcp {
        let applied = apply_redaction_mcp_input(
            request_data,
            verdict.sanitized_text.as_deref(),
            verdict.sanitized_mcp_arguments.as_ref(),
        );
        return redaction_outcome(applied, None);
    }
    let applied = redact_chat_input(
        request_data,
        verdict.sanitized_text.as_deref(),
        verdict.sanitized_messages.as_deref(),
    );
    redaction_outcome(applied, None)
}

// Backs LiteLLM's fn _apply_redaction's output branch: a mismatched surface/response pairing is treated as nothing to rewrite.
pub(crate) fn apply_redaction_output(
    response_surface: ResponseSurface<'_>,
    surface: InspectionType,
    verdict: &CiscoVerdict,
) -> RedactionOutcome {
    match (surface, response_surface) {
        (InspectionType::Mcp, ResponseSurface::McpResponse(mcp_response)) => {
            let update =
                apply_redaction_mcp_output(Some(mcp_response), verdict.sanitized_text.as_deref());
            redaction_outcome(update.as_ref().is_some_and(|value| value.replaced), update)
        }
        (InspectionType::Chat, ResponseSurface::ChatResponse(chat_response)) => redaction_outcome(
            redact_chat_output(
                Some(chat_response),
                verdict.sanitized_text.as_deref(),
                verdict.sanitized_messages.as_deref(),
            ),
            None,
        ),
        (
            InspectionType::Mcp | InspectionType::Chat,
            ResponseSurface::None
            | ResponseSurface::ChatResponse(_)
            | ResponseSurface::McpResponse(_),
        ) => redaction_outcome(false, None),
    }
}

mod tests;
