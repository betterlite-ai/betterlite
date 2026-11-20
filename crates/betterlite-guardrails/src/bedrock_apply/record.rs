use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::catalog::ModelCatalog;

use crate::bedrock_decisions::error::BedrockDecisionError;
use crate::bedrock_decisions::logging::{
    AttemptLog, CallLog, log_attempt, log_failure, log_success,
};
use crate::bedrock_decisions::tracing::Pricing;
use crate::bedrock_records::{Response, Usage};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink,
};

struct SilentSpan;

impl GuardrailSpanSink for SilentSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

static SILENT_SPAN: SilentSpan = SilentSpan;

// everything LiteLLM's three ApplyGuardrail log helpers close over, borrowed for the whole logical call
pub(super) struct LogSite<'call> {
    pub(super) guardrail_provider: &'static str,
    pub(super) base: &'call GuardrailBase,
    pub(super) request_data: &'call mut BTreeMap<String, GuardrailValue>,
    pub(super) log_target: Option<&'call mut GuardrailLogTarget>,
    pub(super) event_type: GuardrailEventHooks,
    pub(super) started: SystemTime,
    pub(super) now: fn() -> SystemTime,
    pub(super) catalog: &'call ModelCatalog,
    pub(super) region: &'call str,
}

// LiteLLM's _log_apply_guardrail_attempt: a blocking call's record sums what earlier chunks already billed
pub(super) fn write_attempt(
    site: &mut LogSite<'_>,
    status_code: u16,
    body: &[u8],
    response: &Response,
    completed: &[Usage],
) -> Result<(), BedrockDecisionError> {
    let call = call_log(site);
    let mut context = record_context(site);
    log_attempt(
        &mut context,
        &call,
        &AttemptLog {
            status_code,
            body,
            json_response: response,
            completed_chunk_usages: completed,
        },
    )
}

pub(super) fn write_success(
    site: &mut LogSite<'_>,
    merged: &Response,
) -> Result<(), BedrockDecisionError> {
    let call = call_log(site);
    let mut context = record_context(site);
    log_success(&mut context, &call, merged)
}

pub(super) fn write_failure(
    site: &mut LogSite<'_>,
    detail: &str,
    completed: &[Usage],
) -> Result<(), BedrockDecisionError> {
    let call = call_log(site);
    let mut context = record_context(site);
    log_failure(&mut context, &call, detail, completed)
}

// the returned record borrows only the catalog and region behind the site, never the site itself
fn call_log<'call>(site: &LogSite<'call>) -> CallLog<'call> {
    CallLog {
        guardrail_provider: site.guardrail_provider,
        event_type: Some(site.event_type),
        start_time: site.started,
        now: (site.now)(),
        pricing: Pricing {
            catalog: site.catalog,
            aws_region_name: Some(site.region),
        },
    }
}

fn record_context<'ctx>(site: &'ctx mut LogSite<'_>) -> GuardrailRecordContext<'ctx> {
    GuardrailRecordContext {
        base: site.base,
        request_data: &mut *site.request_data,
        log_target: site.log_target.as_deref_mut(),
        sink: &SILENT_SPAN,
    }
}
