use betterlite_prompts::arguments::json::Object;

use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::GuardrailRecordContext;
use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_dlp_evaluation::content_check::{ContentCheck, ContentOutcome, check_content};
use crate::purview_graph_client::wire::Activity;

use super::Data;
use super::context::RecordTargets;
use super::error::PurviewHookError;

pub(super) struct ScanRequest<'request> {
    pub user_id: &'request str,
    pub text: &'request str,
    pub activity: Activity,
    pub block_on_violation: bool,
    pub request_data: &'request Object,
}

// LiteLLM joins the scanned segments of one response with this separator before evaluating them.
pub(super) const PART_SEPARATOR: &str = "\n\n---\n\n";

pub(super) struct PartsScan<'request> {
    pub user_id: &'request str,
    pub parts: &'request [String],
    pub block_on_violation: bool,
    pub request_data: &'request Object,
}

pub(super) async fn scan_and_record(
    guardrail: &MicrosoftPurviewDLPGuardrail,
    data: &mut Data,
    scan: &ScanRequest<'_>,
    targets: RecordTargets<'_>,
) -> Result<(), PurviewHookError> {
    let check = ContentCheck {
        user_id: scan.user_id,
        text: scan.text,
        activity: scan.activity,
        request_data: scan.request_data,
        block_on_violation: scan.block_on_violation,
    };
    let outcome = check_content(&guardrail.base, &check).await;
    let mut record_context = GuardrailRecordContext {
        base: targets.base,
        request_data: data,
        log_target: targets.log_target,
        sink: targets.sink,
    };
    if let Some(invocation) = targets.invocation {
        return outcome
            .write_record(invocation, &mut record_context)
            .map(|_response| ())
            .map_err(PurviewHookError::from);
    }
    let ContentOutcome { result, record } = outcome;
    add_guardrail_record(&mut record_context, record).map_err(|source| {
        PurviewHookError::Record {
            source: Box::new(source),
        }
    })?;
    result.map(|_response| ()).map_err(PurviewHookError::from)
}

pub(super) async fn scan_parts(
    guardrail: &MicrosoftPurviewDLPGuardrail,
    data: &mut Data,
    scan: &PartsScan<'_>,
    targets: RecordTargets<'_>,
) -> Result<(), PurviewHookError> {
    let text = scan.parts.join(PART_SEPARATOR);
    let request = ScanRequest {
        user_id: scan.user_id,
        text: &text,
        activity: Activity::DownloadText,
        block_on_violation: scan.block_on_violation,
        request_data: scan.request_data,
    };
    scan_and_record(guardrail, data, &request, targets).await
}
