use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::Object;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_graph_client::user_id_resolution::resolve_logging_user_id;
use crate::purview_graph_client::wire::Activity;

use super::context::RecordTargets;
use super::error::PurviewHookError;
use super::scan::{PartsScan, ScanRequest, scan_and_record, scan_parts};
use super::{Data, conversion, request_text, response_parts};

// The audit needs the guardrail by Arc: the deferred path runs it from another thread while the request returns.
#[derive(Clone)]
pub struct AuditEngine {
    pub guardrail: Arc<MicrosoftPurviewDLPGuardrail>,
    pub record_base: GuardrailBase,
    pub cache: Arc<DualLocalCache>,
    pub sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
}

// What a deferred audit owns: the caller's copies of the hooks' arguments.
#[derive(Debug, Clone)]
pub struct DeferredCall {
    pub kwargs: Data,
    pub result: GuardrailValue,
    pub call_type: Option<CallType>,
}

pub struct AuditCall<'call> {
    pub kwargs: &'call mut Data,
    pub result: &'call GuardrailValue,
    pub call_type: Option<CallType>,
}

impl AuditEngine {
    fn targets(&self) -> RecordTargets<'_> {
        RecordTargets {
            base: &self.record_base,
            invocation: None,
            sink: self.sink.as_ref(),
            log_target: None,
        }
    }

    pub(super) fn audit_blocking(&self, mut call: DeferredCall) {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::error!("Purview audit background logging error: {error}");
                return;
            }
        };
        runtime.block_on(audit_call(
            self,
            &mut call.kwargs,
            &call.result,
            call.call_type,
        ));
    }
}

// LiteLLM wraps each audit call in its own try/except, so a prompt failure never stops the response audit.
async fn audit_call(
    engine: &AuditEngine,
    kwargs: &mut Data,
    result: &GuardrailValue,
    call_type: Option<CallType>,
) {
    let request_data = conversion::object(kwargs);
    let Some(user_id) = resolve_logging_user_id(&request_data) else {
        tracing::debug!("Purview audit: no user_id, skipping");
        return;
    };
    audit_prompt(engine, kwargs, &request_data, &user_id, call_type).await;
    audit_response(engine, kwargs, &request_data, &user_id, result).await;
}

async fn audit_prompt(
    engine: &AuditEngine,
    kwargs: &mut Data,
    request_data: &Object,
    user_id: &str,
    call_type: Option<CallType>,
) {
    if let Err(error) = prompt_scan(engine, kwargs, request_data, user_id, call_type).await {
        tracing::error!("Purview audit logging error (prompt): {error}");
    }
}

async fn prompt_scan(
    engine: &AuditEngine,
    kwargs: &mut Data,
    request_data: &Object,
    user_id: &str,
    call_type: Option<CallType>,
) -> Result<(), PurviewHookError> {
    let Some(text) =
        request_text::audit_prompt_text(kwargs, request_data, call_type, &engine.cache)?
    else {
        return Ok(());
    };
    let scan = ScanRequest {
        user_id,
        text: &text,
        activity: Activity::UploadText,
        block_on_violation: false,
        request_data,
    };
    scan_and_record(&engine.guardrail, kwargs, &scan, engine.targets()).await
}

async fn audit_response(
    engine: &AuditEngine,
    kwargs: &mut Data,
    request_data: &Object,
    user_id: &str,
    result: &GuardrailValue,
) {
    match response_scan(engine, kwargs, request_data, user_id, result).await {
        Ok(()) => {}
        Err(error) => tracing::error!("Purview audit logging error (response): {error}"),
    }
}

async fn response_scan(
    engine: &AuditEngine,
    kwargs: &mut Data,
    request_data: &Object,
    user_id: &str,
    result: &GuardrailValue,
) -> Result<(), PurviewHookError> {
    let parts = response_parts::parts(result)?;
    if parts.is_empty() {
        return Ok(());
    }
    let scan = PartsScan {
        user_id,
        parts: &parts,
        block_on_violation: false,
        request_data,
    };
    scan_parts(&engine.guardrail, kwargs, &scan, engine.targets()).await
}

// Audit never raises: both scans log their failures and the caller gets its arguments back untouched.
pub async fn async_logging_hook(engine: &AuditEngine, call: AuditCall<'_>) {
    audit_call(engine, call.kwargs, call.result, call.call_type).await;
}

mod tests;
