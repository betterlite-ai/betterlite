use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_dlp_evaluation::user_id_blocking::resolve_user_id_for_blocking;

use super::context::HookContext;
use super::error::PurviewHookError;
use super::scan::{PartsScan, scan_parts};
use super::{Data, conversion, response_parts};

// The response comes back untouched; only the DLP scan and its record change `data`.
pub async fn async_post_call_success_hook(
    guardrail: &MicrosoftPurviewDLPGuardrail,
    data: &mut Data,
    response: GuardrailValue,
    context: &mut HookContext<'_>,
) -> Result<GuardrailValue, PurviewHookError> {
    let request_data = conversion::object(data);
    let user_id = resolve_user_id_for_blocking(&guardrail.base, &request_data, &context.identity)?;
    let parts = response_parts::parts(&response)?;
    if !parts.is_empty() {
        let scan = PartsScan {
            user_id: &user_id,
            parts: &parts,
            block_on_violation: true,
            request_data: &request_data,
        };
        let targets = context.record_targets();
        scan_parts(guardrail, data, &scan, targets).await?;
    }
    Ok(response)
}

mod tests;
