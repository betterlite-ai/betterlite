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
use crate::tool_permission_requests::header_mutation::record_applied_guardrail;

use super::RepelloAILifecycle;
use super::stream::{assemble, collect, parse_chunks, relay, replay};
use super::verdict::{log_flagged_verdict, raise_if_blocked, verdict_blocks};
use super::{BLOCKED_MESSAGE, Data, PROMPT_STAGE, RESPONSE_STAGE};

pub struct HookContext<'context> {
    pub suppressed: &'context BTreeSet<String>,
    pub invocation: &'context mut GuardrailInvocation,
    pub sink: &'context (dyn GuardrailSpanSink + Sync),
    pub log_target: Option<&'context mut GuardrailLogTarget>,
}

// The text a non-streamed hook scans: the request's prompt, or the model's response body.
#[derive(Debug, Clone, Copy)]
pub enum HookTarget<'response> {
    Prompt,
    Response(&'response Data),
}

// LiteLLM's async_pre_call_hook for the prompt and async_post_call_success_hook for the response
pub async fn scan_hook(
    guardrail: &RepelloAILifecycle,
    data: &mut Data,
    target: HookTarget<'_>,
    context: &mut HookContext<'_>,
) -> Result<(), GuardrailFailure> {
    let (hook, event_type, stage, skipped) = match target {
        HookTarget::Prompt => (
            "pre_call_hook",
            GuardrailEventHooks::PreCall,
            PROMPT_STAGE,
            "no inspectable prompt text in data",
        ),
        HookTarget::Response(_) => (
            "post_call_success_hook",
            GuardrailEventHooks::PostCall,
            RESPONSE_STAGE,
            "no inspectable response text",
        ),
    };
    tracing::debug!("RepelloAI Argus: {hook}");
    if !guardrail
        .base
        .should_run_guardrail(data, event_type, context.suppressed)?
    {
        return Ok(());
    }
    let extracted = match target {
        HookTarget::Prompt => extract_prompt_text(data),
        HookTarget::Response(response) => extract_response_text(&ResponseSource::Body(response)),
    };
    // B3793's extractors answer None where LiteLLM's `if not text` would see an empty string
    let Some(text) = extracted else {
        tracing::warn!("RepelloAI Argus: {skipped} - skipping.");
        return Ok(());
    };
    let scanned = guardrail.call_analyze(&text, stage, data, context).await?;
    raise_if_blocked(scanned.as_ref())?;
    record_applied(guardrail, data);
    Ok(())
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
    let assembled = assemble(guardrail, &parse_chunks(&chunks)?).await?;
    let Some(text) = assembled.text else {
        let guardrail_name = &guardrail.base.settings.guardrail_name;
        let name = guardrail_name.as_deref().unwrap_or("None");
        tracing::warn!(
            "RepelloAI Argus: no inspectable text in streamed response; skipping scan. guardrail={name} assembled_type={}",
            assembled.kind
        );
        replay(chunks, output).await;
        return Ok(());
    };
    let scanned = guardrail
        .call_analyze(&text, RESPONSE_STAGE, data, context)
        .await?;
    if let Some(analysis) = scanned.as_ref() {
        log_flagged_verdict(analysis);
    }
    if verdict_blocks(scanned.as_ref()) {
        return Err(GuardrailFailure::Other(BLOCKED_MESSAGE.to_owned()));
    }
    record_applied(guardrail, data);
    replay(chunks, output).await;
    Ok(())
}

fn record_applied(guardrail: &RepelloAILifecycle, data: &mut Data) {
    if let Some(name) = guardrail.base.settings.guardrail_name.as_deref() {
        record_applied_guardrail(data, name);
    }
}
