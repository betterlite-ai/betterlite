use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::audit_bridge;
use super::context::HookContext;
use super::data::{Data, MESSAGES, PROMPT, call_id, messages};
use super::dedup::{self, ScanType};
use super::error::{self, HookDataError};
use super::gate;
use super::masking;
use super::prompt;
use super::scan_metadata;
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{GuardrailHttpError, build_error_detail};
use crate::panw_prisma_airs::error_handling::{handle_api_error_with_logging, record_scan_id};
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
use crate::prisma_airs_masking::get_masked_text;

// Mirrors LiteLLM's fn async_pre_call_hook.
pub async fn pre_call_hook(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    audit: &mut GuardrailAudit,
    context: &HookContext<'_>,
) -> Result<(), GuardrailHttpError> {
    tracing::info!("PANW Prisma AIRS: Running pre-call prompt scan");
    let outcome = run_pre_call(guardrail, data, audit, context).await;
    audit_bridge::flush(data, audit, guardrail.settings.guardrail_name.as_str());
    outcome
}

async fn run_pre_call(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    audit: &mut GuardrailAudit,
    context: &HookContext<'_>,
) -> Result<(), GuardrailHttpError> {
    let should_run = gate::should_run_guardrail(data, GuardrailEventHooks::PreCall, context)
        .map_err(|source| error::internal(&source, Direction::Request, guardrail))?;
    if !should_run {
        return Ok(());
    }
    if dedup::check_and_mark_scanned(data, ScanType::Pre, context.new_id)
        .map_err(|source| error::internal(&source, Direction::Request, guardrail))?
    {
        return Ok(());
    }
    let start_time = (guardrail.now)();
    let prompt_text = prompt::extract_prompt_from_request(data)
        .map_err(|source| error::internal(&source, Direction::Request, guardrail))?;
    if prompt_text.is_empty() {
        tracing::warn!(
            "PANW Prisma AIRS: No user prompt found in request (checked 'messages' and 'prompt' fields)"
        );
        return Ok(());
    }
    let Some(verdict) = scan_prompt(guardrail, data, &prompt_text, start_time, audit).await? else {
        return Ok(());
    };
    audit_bridge::record_information(guardrail, audit, &verdict, start_time, Direction::Request);
    record_scan_id(guardrail, audit, &verdict, Direction::Request);
    apply_prompt_verdict(guardrail, data, audit, &verdict)
}

// Ok(None) means a transient error was waved through by _handle_api_error_with_logging.
async fn scan_prompt(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    prompt_text: &str,
    start_time: SystemTime,
    audit: &mut GuardrailAudit,
) -> Result<Option<ScanVerdict>, GuardrailHttpError> {
    let metadata = scan_metadata::scan_metadata(data)
        .map_err(|source| error::internal(&source, Direction::Request, guardrail))?;
    let verdict = call_panw_api(
        guardrail,
        ScanRequest {
            content: prompt_text,
            direction: Direction::Request,
            metadata: Some(&metadata),
            call_id: call_id(data),
            tool_event: None,
        },
    )
    .await;
    if verdict.failure.is_some() {
        handle_api_error_with_logging(guardrail, &verdict, audit, start_time, Direction::Request)?;
        return Ok(None);
    }
    Ok(Some(verdict))
}

fn apply_prompt_verdict(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    audit: &mut GuardrailAudit,
    verdict: &ScanVerdict,
) -> Result<(), GuardrailHttpError> {
    let masked = get_masked_text(&verdict.masks, false).filter(|text| !text.is_empty());
    let category = verdict.category.as_deref().unwrap_or("unknown");
    match verdict.action {
        Action::Allow => {
            apply_prompt_masking(guardrail, data, masked)?;
            if masked.is_some() {
                tracing::info!(
                    "PANW Prisma AIRS: Prompt allowed with masking (Category: {category})"
                );
            } else {
                tracing::info!("PANW Prisma AIRS: Prompt allowed (Category: {category})");
            }
            audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
            Ok(())
        }
        Action::Block if masked.is_some() && guardrail.settings.masking.content.request_content => {
            apply_prompt_masking(guardrail, data, masked)?;
            tracing::warn!(
                "PANW Prisma AIRS: Prompt blocked but masked instead (mask_request_content=True)"
            );
            audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
            Ok(())
        }
        Action::Block => {
            let detail = build_error_detail(guardrail, verdict, Direction::Request);
            tracing::warn!("PANW Prisma AIRS: {}", detail.error.message);
            Err(GuardrailHttpError {
                status: 400,
                detail: Box::new(detail),
            })
        }
    }
}

fn apply_prompt_masking(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    masked: Option<&str>,
) -> Result<(), GuardrailHttpError> {
    let Some(text) = masked else {
        return Ok(());
    };
    let not_a_list = data
        .get(MESSAGES)
        .is_some_and(|value| !matches!(value, GuardrailValue::Array(_)));
    if not_a_list {
        return Err(error::internal(
            &HookDataError::NotList { field: MESSAGES },
            Direction::Request,
            guardrail,
        ));
    }
    if messages(data).is_some_and(|items| !items.is_empty()) {
        return masking::apply_masking_to_messages(data, text)
            .map_err(|source| error::internal(&source, Direction::Request, guardrail));
    }
    if data.contains_key(PROMPT) {
        data.insert(PROMPT.to_owned(), GuardrailValue::Text(text.to_owned()));
    }
    Ok(())
}
