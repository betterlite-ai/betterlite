use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::Object;
use tokio::runtime::Builder;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_graph_client::user_id_resolution::resolve_logging_user_id;
use crate::purview_graph_client::wire::Activity;

use super::context::RecordTargets;
use super::scan::{PART_SEPARATOR, ScanRequest, scan_and_record};
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

impl AuditEngine {
    pub(super) fn audit_blocking(&self, mut call: DeferredCall) {
        let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
            tracing::error!("Purview audit background logging error: no runtime");
            return;
        };
        runtime.block_on(async_logging_hook(
            self,
            &mut call.kwargs,
            &call.result,
            call.call_type,
        ));
    }
}

// What one audit call borrows from the request; the prompt and response audits share it.
struct Audit<'audit> {
    engine: &'audit AuditEngine,
    kwargs: &'audit mut Data,
    request_data: &'audit Object,
    user_id: &'audit str,
}

// LiteLLM wraps each audit call in its own try/except, so a prompt failure never stops the response audit.
pub async fn async_logging_hook(
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
    let prompt = prompt_for_audit(engine, kwargs, &request_data, call_type);
    let response = parts_for_audit(result);
    let mut audit = Audit {
        engine,
        kwargs,
        request_data: &request_data,
        user_id: &user_id,
    };
    if let Some(text) = prompt {
        audit_text(&mut audit, Activity::UploadText, "prompt", &text).await;
    }
    if let Some(text) = response {
        audit_text(&mut audit, Activity::DownloadText, "response", &text).await;
    }
}

fn prompt_for_audit(
    engine: &AuditEngine,
    kwargs: &Data,
    request_data: &Object,
    call_type: Option<CallType>,
) -> Option<String> {
    match request_text::prompt_text(kwargs, request_data, call_type, &engine.cache, false) {
        Ok(text) => text,
        Err(error) => {
            tracing::error!("Purview audit logging error (prompt): {error}");
            None
        }
    }
}

fn parts_for_audit(result: &GuardrailValue) -> Option<String> {
    match response_parts::parts(result) {
        Ok(parts) if !parts.is_empty() => Some(parts.join(PART_SEPARATOR)),
        Ok(_) => None,
        Err(error) => {
            tracing::error!("Purview audit logging error (response): {error}");
            None
        }
    }
}

async fn audit_text(audit: &mut Audit<'_>, activity: Activity, label: &str, text: &str) {
    let scan = ScanRequest {
        user_id: audit.user_id,
        text,
        activity,
        block_on_violation: false,
        request_data: audit.request_data,
    };
    let targets = RecordTargets {
        base: &audit.engine.record_base,
        invocation: None,
        sink: audit.engine.sink.as_ref(),
        log_target: None,
    };
    if let Err(error) = scan_and_record(&audit.engine.guardrail, audit.kwargs, &scan, targets).await
    {
        tracing::error!("Purview audit logging error ({label}): {error}");
    }
}

mod tests;
