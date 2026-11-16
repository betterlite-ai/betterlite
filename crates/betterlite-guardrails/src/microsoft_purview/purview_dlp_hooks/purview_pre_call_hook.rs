use betterlite_core::routes::CallType;

use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_dlp_evaluation::user_id_blocking::resolve_user_id_for_blocking;
use crate::purview_graph_client::wire::Activity;

use super::context::{HookContext, RecordTargets};
use super::error::PurviewHookError;
use super::scan::{ScanRequest, scan_and_record};
use super::{Data, conversion, request_text};

pub async fn async_pre_call_hook(
    guardrail: &MicrosoftPurviewDLPGuardrail,
    data: &mut Data,
    call_type: Option<CallType>,
    context: &mut HookContext<'_>,
) -> Result<(), PurviewHookError> {
    let request_data = conversion::object(data);
    let user_id = resolve_user_id_for_blocking(&guardrail.base, &request_data, &context.identity)?;
    let Some(text) =
        request_text::prompt_text(data, &request_data, call_type, &context.cache, true)?
    else {
        return Ok(());
    };
    let scan = ScanRequest {
        user_id: &user_id,
        text: &text,
        activity: Activity::UploadText,
        block_on_violation: true,
        request_data: &request_data,
    };
    let targets = RecordTargets {
        base: context.record_base,
        invocation: context.invocation.as_deref_mut(),
        sink: context.sink,
        log_target: context.log_target.as_deref_mut(),
    };
    scan_and_record(guardrail, data, &scan, targets).await
}

mod tests;
