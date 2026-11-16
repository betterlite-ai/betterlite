use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use tokio::sync::mpsc;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::repello_content::prompt::extract_prompt_text;
use crate::repello_content::response::{ResponseSource, extract_response_text};
use crate::repello_content::wire::RepelloAIAnalyzeResponse;
use crate::tool_permission_requests::header_mutation::record_applied_guardrail;

use super::RepelloAILifecycle;
use super::analyze::{RepelloStage, Scan};
use super::stream::{assemble, collect, parse_chunks, relay, replay};
use super::verdict::{log_flagged_verdict, raise_if_blocked, verdict_blocks};
use super::{BLOCKED_MESSAGE, Data};

pub struct HookContext<'context> {
    pub suppressed: &'context BTreeSet<String>,
    pub invocation: &'context mut GuardrailInvocation,
    pub sink: &'context dyn GuardrailSpanSink,
    pub log_target: Option<&'context mut GuardrailLogTarget>,
}

// LiteLLM's async_pre_call_hook
pub async fn pre_call_hook(
    guardrail: &RepelloAILifecycle,
    data: &mut Data,
    context: &mut HookContext<'_>,
) -> Result<(), GuardrailFailure> {
    tracing::debug!("RepelloAI Argus: pre_call_hook");
    if !guardrail.base.should_run_guardrail(
        data,
        GuardrailEventHooks::PreCall,
        context.suppressed,
    )? {
        return Ok(());
    }
    let text = extract_prompt_text(data).filter(|text| !text.is_empty());
    let Some(text) = text else {
        tracing::warn!("RepelloAI Argus: no inspectable prompt text in data - skipping.");
        return Ok(());
    };
    let scan = Scan {
        text: &text,
        stage: RepelloStage::Prompt,
        event_type: GuardrailEventHooks::PreCall,
    };
    inspect(guardrail, scan, data, context).await
}

// LiteLLM's async_post_call_success_hook
pub async fn post_call_success_hook(
    guardrail: &RepelloAILifecycle,
    data: &mut Data,
    response: &Data,
    context: &mut HookContext<'_>,
) -> Result<(), GuardrailFailure> {
    tracing::debug!("RepelloAI Argus: post_call_success_hook");
    if !guardrail.base.should_run_guardrail(
        data,
        GuardrailEventHooks::PostCall,
        context.suppressed,
    )? {
        return Ok(());
    }
    let text =
        extract_response_text(&ResponseSource::Body(response)).filter(|text| !text.is_empty());
    let Some(text) = text else {
        tracing::warn!("RepelloAI Argus: no inspectable response text - skipping.");
        return Ok(());
    };
    let scan = Scan {
        text: &text,
        stage: RepelloStage::Response,
        event_type: GuardrailEventHooks::PostCall,
    };
    inspect(guardrail, scan, data, context).await
}

// LiteLLM's async_post_call_streaming_iterator_hook: every chunk is held back until the scan passes
pub async fn streaming_iterator_hook(
    guardrail: &RepelloAILifecycle,
    data: &mut Data,
    context: &mut HookContext<'_>,
    response: mpsc::Receiver<GuardrailValue>,
    output: mpsc::Sender<GuardrailValue>,
) -> Result<(), GuardrailFailure> {
    if !guardrail.base.should_run_guardrail(
        data,
        GuardrailEventHooks::PostCall,
        context.suppressed,
    )? {
        relay(response, output).await;
        return Ok(());
    }
    let chunks = collect(response).await;
    let parsed = parse_chunks(&chunks)?;
    let assembled = assemble(guardrail, &parsed).await?;
    let Some(text) = assembled.text else {
        warn_no_stream_text(guardrail, assembled.kind);
        replay(chunks, output).await;
        return Ok(());
    };
    let scan = Scan {
        text: &text,
        stage: RepelloStage::Response,
        event_type: GuardrailEventHooks::PostCall,
    };
    let scanned = guardrail.call_analyze(scan, data, context).await?;
    finish_stream_scan(guardrail, scanned.as_ref(), data)?;
    replay(chunks, output).await;
    Ok(())
}

fn warn_no_stream_text(guardrail: &RepelloAILifecycle, kind: &str) {
    let name = guardrail
        .base
        .settings
        .guardrail_name
        .as_deref()
        .unwrap_or("None");
    tracing::warn!(
        "RepelloAI Argus: no inspectable text in streamed response; skipping scan. guardrail={name} assembled_type={kind}"
    );
}

// LiteLLM logs a flagged verdict, blocks an unrecognized or blocked one, and only then records the guardrail as applied.
fn finish_stream_scan(
    guardrail: &RepelloAILifecycle,
    scanned: Option<&RepelloAIAnalyzeResponse>,
    data: &mut Data,
) -> Result<(), GuardrailFailure> {
    if let Some(scanned) = scanned {
        log_flagged_verdict(scanned);
    }
    if verdict_blocks(scanned) {
        return Err(GuardrailFailure::Other(BLOCKED_MESSAGE.to_owned()));
    }
    if let Some(name) = guardrail.base.settings.guardrail_name.as_deref() {
        record_applied_guardrail(data, name);
    }
    Ok(())
}

async fn inspect(
    guardrail: &RepelloAILifecycle,
    scan: Scan<'_>,
    data: &mut Data,
    context: &mut HookContext<'_>,
) -> Result<(), GuardrailFailure> {
    let scanned = guardrail.call_analyze(scan, data, context).await?;
    raise_if_blocked(scanned.as_ref())?;
    if let Some(name) = guardrail.base.settings.guardrail_name.as_deref() {
        record_applied_guardrail(data, name);
    }
    Ok(())
}
