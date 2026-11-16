use std::time::SystemTime;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::audit_bridge::{self, ScanRecord};
use super::context::HookContext;
use super::data::{Data, call_id};
use super::dedup::{self, ScanType};
use super::error;
use super::gate;
use super::response::{CallResponse, extraction_view};
use super::scan_metadata;
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{GuardrailHttpError, build_error_detail};
use crate::panw_prisma_airs::error_handling::{handle_api_error_with_logging, record_scan_id};
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
use crate::prisma_airs_extraction::extract_response_text;
use crate::prisma_airs_masking::{Response, get_masked_text, mask_response};

// Mirrors LiteLLM's fn async_post_call_success_hook.
pub async fn post_call_hook(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    response: CallResponse<'_>,
    audit: &mut GuardrailAudit,
    context: &HookContext<'_>,
) -> Result<(), GuardrailHttpError> {
    let CallResponse::Model(response) = response else {
        return Ok(());
    };
    tracing::info!("PANW Prisma AIRS: Running post-call response scan");
    let outcome = run_post_call(guardrail, data, response, audit, context).await;
    audit_bridge::flush(data, audit, guardrail.settings.guardrail_name.as_str());
    outcome
}

async fn run_post_call(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    response: &mut Response,
    audit: &mut GuardrailAudit,
    context: &HookContext<'_>,
) -> Result<(), GuardrailHttpError> {
    let should_run = gate::should_run_guardrail(data, GuardrailEventHooks::PostCall, context)
        .map_err(|source| error::internal(&source, Direction::Response, guardrail))?;
    if !should_run {
        return Ok(());
    }
    if dedup::check_and_mark_scanned(data, ScanType::Post, context.new_id)
        .map_err(|source| error::internal(&source, Direction::Response, guardrail))?
    {
        return Ok(());
    }
    let start_time = (guardrail.now)();
    let response_text = extract_response_text(&extraction_view(response));
    if response_text.is_empty() {
        tracing::warn!("PANW Prisma AIRS: No response content found to scan");
        return Ok(());
    }
    let Some(verdict) = scan_response(guardrail, data, &response_text, start_time, audit).await?
    else {
        return Ok(());
    };
    audit_bridge::record_information(
        guardrail,
        audit,
        ScanRecord {
            verdict: &verdict,
            start_time,
            direction: Direction::Response,
        },
    );
    record_scan_id(guardrail, audit, &verdict, Direction::Response);
    apply_response_verdict(guardrail, response, audit, &verdict)
}

// Ok(None) means a transient error was waved through by _handle_api_error_with_logging.
async fn scan_response(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    response_text: &str,
    start_time: SystemTime,
    audit: &mut GuardrailAudit,
) -> Result<Option<ScanVerdict>, GuardrailHttpError> {
    let metadata = scan_metadata::scan_metadata(data)
        .map_err(|source| error::internal(&source, Direction::Response, guardrail))?;
    let verdict = call_panw_api(
        guardrail,
        ScanRequest {
            content: response_text,
            direction: Direction::Response,
            metadata: Some(&metadata),
            call_id: call_id(data),
            tool_event: None,
        },
    )
    .await;
    if verdict.failure.is_some() {
        handle_api_error_with_logging(guardrail, &verdict, audit, start_time, Direction::Response)?;
        return Ok(None);
    }
    Ok(Some(verdict))
}

fn apply_response_verdict(
    guardrail: &PanwPrismaAirs,
    response: &mut Response,
    audit: &mut GuardrailAudit,
    verdict: &ScanVerdict,
) -> Result<(), GuardrailHttpError> {
    let masked = get_masked_text(&verdict.masks, true).filter(|text| !text.is_empty());
    let category = verdict.category.as_deref().unwrap_or("unknown");
    match (verdict.action, masked) {
        (Action::Allow, Some(text)) => {
            mask_response(response, text)
                .map_err(|source| error::internal(&source, Direction::Response, guardrail))?;
            tracing::info!(
                "PANW Prisma AIRS: Response allowed with masking (Category: {category})"
            );
            audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
            Ok(())
        }
        (Action::Allow, None) => {
            tracing::info!("PANW Prisma AIRS: Response allowed (Category: {category})");
            audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
            Ok(())
        }
        (Action::Block, Some(text)) if guardrail.settings.masking.content.response_content => {
            mask_response(response, text)
                .map_err(|source| error::internal(&source, Direction::Response, guardrail))?;
            tracing::warn!(
                "PANW Prisma AIRS: Response blocked but masked instead (mask_response_content=True)"
            );
            audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
            Ok(())
        }
        (Action::Block, _) => {
            let detail = build_error_detail(guardrail, verdict, Direction::Response);
            tracing::warn!("PANW Prisma AIRS: {}", detail.error.message);
            Err(GuardrailHttpError {
                status: 400,
                detail: Box::new(detail),
            })
        }
    }
}
