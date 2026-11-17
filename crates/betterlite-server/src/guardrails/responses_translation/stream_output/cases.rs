use std::collections::BTreeMap;

use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::Object;
use serde_json::Value;

use crate::guardrails::responses_translation::input_provenance::is_tool_call_item;
use crate::guardrails::responses_translation::output_rewrite::apply_texts_to_output;
use crate::guardrails::responses_translation::stream_scan::integer_field;
use crate::guardrails::responses_translation::stream_sync;

use super::error::StreamOutputError;
use super::{GuardrailAnswer, StreamContext, scan_envelope, terminal_answer, tool_call_inputs};

fn deliver_terminal_rewrites(
    response: &mut Value,
    earlier: &mut [Value],
    mappings: &[(usize, usize)],
    scanned: &[String],
    rewritten: &[String],
    answer: &GuardrailAnswer,
    guardrail_name: Option<&str>,
) -> Result<(), StreamOutputError> {
    let mut rewrites = BTreeMap::new();
    for ((scanned_text, mapping), rewritten_text) in scanned.iter().zip(mappings).zip(rewritten) {
        if scanned_text != rewritten_text {
            rewrites.insert(*mapping, rewritten_text.clone());
        }
    }
    // ast-grep-ignore: no-long-names
    stream_sync::sync_stream_events_with_rewrites(earlier, &rewrites);
    let mut empty: [Value; 0] = [];
    let output = match response.get_mut("output") {
        Some(Value::Array(items)) => items.as_mut_slice(),
        Some(_) | None => empty.as_mut_slice(),
    };
    // ast-grep-ignore: no-long-names
    stream_sync::deliver_ended_stream_tool_call_rewrites(
        earlier,
        output,
        &answer.pre,
        &answer.post,
        guardrail_name.unwrap_or("unknown"),
    )
    .map_err(StreamOutputError::from)
}

pub(super) async fn terminal_scan<G>(
    last: &mut Value,
    earlier: &mut [Value],
    guardrail: &G,
    context: &StreamContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<bool, StreamOutputError>
where
    G: GuardrailContract,
{
    let Some(response) = last.get_mut("response") else {
        return Ok(false);
    };
    if !response.is_object() {
        return Ok(false);
    }
    let scan = scan_envelope(response)?;
    if scan.extracted.texts.is_empty() && scan.extracted.tool_calls.is_empty() {
        return Ok(false);
    }
    let answer = terminal_answer(&scan, guardrail, context, logging_obj, request_data).await?;
    let texts = answer.inputs.texts.as_deref().unwrap_or_default();
    apply_texts_to_output(response, texts, &scan.extracted.task_mappings)?;
    if context.deliver {
        deliver_terminal_rewrites(
            response,
            earlier,
            &scan.extracted.task_mappings,
            &scan.extracted.texts,
            texts,
            &answer,
            context.guardrail_name,
        )?;
    }
    Ok(true)
}

pub(super) async fn item_done_scan<G>(
    last: &Value,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<(), StreamOutputError>
where
    G: GuardrailContract,
{
    let Some(item) = last.get("item") else {
        return Ok(());
    };
    let Some(fields) = item.as_object() else {
        return Ok(());
    };
    if !is_tool_call_item(item) {
        return Ok(());
    }
    let index = integer_field(last, "output_index").unwrap_or_default();
    let inputs = tool_call_inputs(fields, index)?;
    let mut fresh = Object::new();
    let prepared = request_data.unwrap_or(&mut fresh);
    guardrail
        .apply_guardrail(inputs, prepared, GuardrailInputType::Response, logging_obj)
        .await?;
    Ok(())
}
