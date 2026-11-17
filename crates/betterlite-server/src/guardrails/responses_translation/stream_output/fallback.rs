use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use serde_json::Value;

use crate::guardrails::responses_translation::stream_scan::streaming_string_so_far;
use crate::guardrails::responses_translation::stream_sync;

use super::StreamContext;
use super::error::StreamOutputError;

fn fallback_inputs(text: String, model: Option<String>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(vec![text]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model,
        stream_holdback_chars: None,
    }
}

fn changed_text<'texts>(texts: Option<&'texts [String]>, scanned: &str) -> Option<&'texts str> {
    match texts {
        Some([only]) if only.as_str() == scanned => None,
        Some([first, ..]) => Some(first.as_str()),
        Some([]) | None => None,
    }
}

pub(super) async fn fallback_scan<G>(
    events: &mut [Value],
    guardrail: &G,
    context: &StreamContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<(), StreamOutputError>
where
    G: GuardrailContract,
{
    let string_so_far = streaming_string_so_far(events);
    if string_so_far.is_empty() {
        return Ok(());
    }
    let model = events
        .last()
        .and_then(|event| event.get("response"))
        .and_then(super::model_of);
    let inputs = fallback_inputs(string_so_far.clone(), model);
    let mut fresh = Object::new();
    let prepared = request_data.unwrap_or(&mut fresh);
    let answer = guardrail
        .apply_guardrail(inputs, prepared, GuardrailInputType::Response, logging_obj)
        .await?;
    match changed_text(answer.texts.as_deref(), &string_so_far) {
        Some(rewritten) if context.deliver => {
            // ast-grep-ignore: no-long-names
            stream_sync::spread_text_rewrite_over_stream_events(
                events,
                rewritten,
                context.guardrail_name.unwrap_or("unknown"),
            )?;
            Ok(())
        }
        Some(_) | None => Ok(()),
    }
}
