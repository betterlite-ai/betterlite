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
    let text =
        match request_text::prompt_text(kwargs, request_data, call_type, &engine.cache, false) {
            Ok(text) => text,
            Err(error) => {
                tracing::error!("Purview audit logging error (prompt): {error}");
                return;
            }
        };
    let Some(text) = text else {
        return;
    };
    let scan = ScanRequest {
        user_id,
        text: &text,
        activity: Activity::UploadText,
        block_on_violation: false,
        request_data,
    };
    let targets = RecordTargets {
        base: &engine.record_base,
        invocation: None,
        sink: engine.sink.as_ref(),
        log_target: None,
    };
    if let Err(error) = scan_and_record(&engine.guardrail, kwargs, &scan, targets).await {
        tracing::error!("Purview audit logging error (prompt): {error}");
    }
}

async fn audit_response(
    engine: &AuditEngine,
    kwargs: &mut Data,
    request_data: &Object,
    user_id: &str,
    result: &GuardrailValue,
) {
    let parts = match response_parts::parts(result) {
        Ok(parts) => parts,
        Err(error) => {
            tracing::error!("Purview audit logging error (response): {error}");
            return;
        }
    };
    if parts.is_empty() {
        return;
    }
    let scan = PartsScan {
        user_id,
        parts: &parts,
        block_on_violation: false,
        request_data,
    };
    let targets = RecordTargets {
        base: &engine.record_base,
        invocation: None,
        sink: engine.sink.as_ref(),
        log_target: None,
    };
    if let Err(error) = scan_parts(&engine.guardrail, kwargs, &scan, targets).await {
        tracing::error!("Purview audit logging error (response): {error}");
    }
}

mod tests;
