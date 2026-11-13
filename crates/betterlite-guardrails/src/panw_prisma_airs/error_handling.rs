use std::time::SystemTime;

use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use indexmap::IndexMap;

use super::audit::{Direction, GuardrailAudit, GuardrailInformation, GuardrailStatus};
use super::construct::PanwPrismaAirs;
use super::error_detail::{ErrorBody, ErrorCode, ErrorDetail, ErrorType, GuardrailHttpError};
use super::verdict::{Failure, ScanVerdict};

const PROVIDER_NAME: &str = "panw_prisma_airs";

// Mirrors LiteLLM's fn _record_scan_id.
pub fn record_scan_id(
    guardrail: &PanwPrismaAirs,
    audit: &mut GuardrailAudit,
    verdict: &ScanVerdict,
    direction: Direction,
) {
    audit.add_scan_id(
        verdict.scan_id.as_deref(),
        &guardrail.settings.guardrail_name,
        PROVIDER_NAME,
        direction.stage(),
    );
}

// Mirrors LiteLLM's fn _handle_api_error_with_logging; Ok means the request goes through unscanned.
pub fn handle_api_error_with_logging(
    guardrail: &PanwPrismaAirs,
    verdict: &ScanVerdict,
    audit: &mut GuardrailAudit,
    start_time: SystemTime,
    direction: Direction,
) -> Result<(), GuardrailHttpError> {
    let end_time = (guardrail.now)();
    audit.information.push(GuardrailInformation {
        provider: PROVIDER_NAME.to_owned(),
        status: GuardrailStatus::GuardrailFailedToRespond,
        response: verdict.summary(),
        start_time,
        end_time,
        duration: end_time.duration_since(start_time).unwrap_or_default(),
        event_type: direction.stage(),
    });
    let category = verdict.category.as_deref().unwrap_or("api_error");
    if verdict.failure == Some(Failure::Permanent) {
        return Err(scan_failure(
            guardrail,
            category == "config_error",
            category,
        ));
    }
    if verdict.failure == Some(Failure::Transient)
        && guardrail.settings.fallback_on_error == FallbackAction::Allow
    {
        let side = match direction {
            Direction::Response => "response",
            Direction::Request => "request",
        };
        tracing::warn!(
            "PANW Prisma AIRS: Allowing {side} without scanning (fallback_on_error='allow', error: {category})"
        );
        audit.add_applied_guardrail(&format!("{}:unscanned", guardrail.settings.guardrail_name));
        return Ok(());
    }
    Err(scan_failure(guardrail, false, category))
}

// A configuration error is told apart from a scan failure; both are a 500 that blocks the call.
fn scan_failure(guardrail: &PanwPrismaAirs, is_config: bool, category: &str) -> GuardrailHttpError {
    let (message, kind, code) = if is_config {
        (
            "Security scan failed - configuration error",
            ErrorType::GuardrailConfigError,
            ErrorCode::PanwPrismaAirsConfigError,
        )
    } else {
        (
            "Security scan failed - request blocked for safety",
            ErrorType::GuardrailScanError,
            ErrorCode::PanwPrismaAirsScanFailed,
        )
    };
    GuardrailHttpError {
        status: 500,
        detail: Box::new(ErrorDetail {
            error: ErrorBody {
                action: None,
                scan_fields: IndexMap::new(),
                message: message.to_owned(),
                kind,
                code,
                guardrail: guardrail.settings.guardrail_name.clone(),
                category: category.to_owned(),
            },
        }),
    }
}
