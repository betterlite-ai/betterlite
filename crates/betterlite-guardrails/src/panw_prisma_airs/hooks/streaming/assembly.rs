use std::time::SystemTime;

use super::StreamError;
use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{GuardrailHttpError, build_error_detail};
use crate::panw_prisma_airs::hooks::data::{Data, call_id};
use crate::panw_prisma_airs::hooks::response::extraction_view;
use crate::panw_prisma_airs::hooks::scan_metadata;
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
use crate::prisma_airs_extraction::extract_response_text;
use crate::prisma_airs_masking::{Response, get_masked_text, mask_response};

#[derive(Debug)]
pub struct StreamScan {
    pub modified: bool,
    pub verdict: ScanVerdict,
}

// Mirrors LiteLLM's fn _scan_and_process_streaming_response; a scan failure returns its verdict to the caller.
pub async fn scan_streaming_response(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    response: &mut Response,
    start_time: SystemTime,
) -> Result<StreamScan, StreamError> {
    let response_text = extract_response_text(&extraction_view(response));
    if response_text.trim().is_empty() {
        tracing::info!("PANW Prisma AIRS: No content to scan in streaming response");
        return Ok(StreamScan {
            modified: false,
            verdict: ScanVerdict::synthetic(Action::Allow, "no_content", None),
        });
    }
    let metadata =
        scan_metadata::scan_metadata(data).map_err(|source| StreamError::Data { source })?;
    let verdict = call_panw_api(
        guardrail,
        ScanRequest {
            content: &response_text,
            direction: Direction::Response,
            metadata: Some(&metadata),
            call_id: call_id(data),
            tool_event: None,
        },
    )
    .await;
    if verdict.failure.is_some() {
        return Ok(StreamScan {
            modified: false,
            verdict,
        });
    }
    let modified = apply_masking(guardrail, response, &verdict)?;
    Ok(StreamScan { modified, verdict })
}

// The masking branch of _scan_and_process_streaming_response; errors leave the assembled response for the frame.
fn apply_masking(
    guardrail: &PanwPrismaAirs,
    response: &mut Response,
    verdict: &ScanVerdict,
) -> Result<bool, StreamError> {
    let masked = get_masked_text(&verdict.masks, true).filter(|text| !text.is_empty());
    let category = verdict.category.as_deref().unwrap_or("unknown");
    match (verdict.action, masked) {
        (Action::Allow, Some(text)) => {
            mask_response(response, text)?;
            tracing::info!(
                "PANW Prisma AIRS: Streaming response allowed with masking (Category: {category})"
            );
            Ok(true)
        }
        (Action::Allow, None) => {
            tracing::info!("PANW Prisma AIRS: Streaming response allowed (Category: {category})");
            Ok(false)
        }
        (Action::Block, Some(text)) if guardrail.settings.masking.content.response_content => {
            mask_response(response, text)?;
            tracing::warn!(
                "PANW Prisma AIRS: Streaming response blocked but masked instead (mask_response_content=True)"
            );
            Ok(true)
        }
        (Action::Block, _) => {
            let detail = build_error_detail(guardrail, verdict, Direction::Response);
            tracing::warn!("PANW Prisma AIRS: {}", detail.error.message);
            Err(StreamError::Http(GuardrailHttpError {
                status: 400,
                detail: Box::new(detail),
            }))
        }
    }
}
