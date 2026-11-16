#![expect(
    clippy::disallowed_types,
    clippy::too_many_arguments,
    reason = "Responses stream events are provider JSON that betterlite does not model, and the streaming entry point keeps LiteLLM's argument list"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_chat_history::tool_fields::output_tool_fields;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use serde_json::{Map, Value};

use super::input_provenance::is_tool_call_item;
use super::output_rewrite::error::OutputRewriteError;
use super::output_rewrite::{ExtractedOutput, apply_texts_to_output, extract_output};
use super::stream_scan::{TERMINAL_ENVELOPE_EVENT_TYPES, integer_field, streaming_string_so_far};
use super::stream_sync;
use super::tool_call_identity::{ToolCallShape, post_guardrail_shapes, tool_call_shapes};
use crate::guardrails::response_context::add_response_context;
use error::StreamOutputError;

struct StreamContext<'ctx> {
    guardrail_name: Option<&'ctx str>,
    user_api_key_dict: Option<&'ctx Object>,
    deliver: bool,
}

struct EnvelopeScan {
    extracted: ExtractedOutput,
    model: Option<String>,
    response: GuardrailValue,
}

struct GuardrailAnswer {
    inputs: GuardrailApiInputs,
    pre: Vec<ToolCallShape>,
    post: Vec<ToolCallShape>,
}

fn event_type(event: &Value) -> Option<&str> {
    event.get("type").and_then(Value::as_str)
}

fn model_of(response: &Value) -> Option<String> {
    response
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

fn scan_envelope(response: &Value) -> Result<EnvelopeScan, StreamOutputError> {
    let empty: &[Value] = &[];
    let outputs = response
        .get("output")
        .and_then(Value::as_array)
        .map_or(empty, Vec::as_slice);
    let value =
        serde_json::from_value(response.clone()).map_err(|source| StreamOutputError::Convert {
            target: "response",
            source,
        })?;
    Ok(EnvelopeScan {
        extracted: extract_output(outputs)?,
        model: model_of(response),
        response: value,
    })
}

fn guardrail_values(
    values: &[Value],
    target: &'static str,
) -> Result<Vec<GuardrailValue>, StreamOutputError> {
    let value = serde_json::to_value(values)
        .map_err(|source| StreamOutputError::Convert { target, source })?;
    serde_json::from_value(value).map_err(|source| StreamOutputError::Convert { target, source })
}

fn json_values(
    values: &[GuardrailValue],
    target: &'static str,
) -> Result<Vec<Value>, StreamOutputError> {
    let value = serde_json::to_value(values)
        .map_err(|source| StreamOutputError::Convert { target, source })?;
    serde_json::from_value(value).map_err(|source| StreamOutputError::Convert { target, source })
}

fn post_shapes(
    answer: &GuardrailApiInputs,
    pre: &[ToolCallShape],
    guardrail_name: Option<&str>,
) -> Result<Vec<ToolCallShape>, StreamOutputError> {
    let Some(calls) = answer.tool_calls.as_ref() else {
        return Ok(post_guardrail_shapes(None, pre, guardrail_name));
    };
    let returned = json_values(calls, "returned tool calls")?;
    Ok(post_guardrail_shapes(Some(&returned), pre, guardrail_name))
}

fn tool_call_inputs(
    item: &Map<String, Value>,
    index: usize,
) -> Result<GuardrailApiInputs, StreamOutputError> {
    let chunk = output_tool_fields(item, index).map_err(OutputRewriteError::from)?;
    let value = serde_json::to_value(&chunk).map_err(|source| StreamOutputError::Convert {
        target: "tool call",
        source,
    })?;
    let tool_call: GuardrailValue =
        serde_json::from_value(value).map_err(|source| StreamOutputError::Convert {
            target: "tool call",
            source,
        })?;
    Ok(GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: Some(vec![tool_call]),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    })
}

async fn terminal_answer<G>(
    scan: &EnvelopeScan,
    guardrail: &G,
    context: &StreamContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<GuardrailAnswer, StreamOutputError>
where
    G: GuardrailContract,
{
    let mut fresh = Object::new();
    let prepared = request_data.unwrap_or(&mut fresh);
    add_response_context(prepared, &scan.response, context.user_api_key_dict);
    let tool_calls = if scan.extracted.tool_calls.is_empty() {
        None
    } else {
        Some(guardrail_values(&scan.extracted.tool_calls, "tool calls")?)
    };
    let inputs = GuardrailApiInputs {
        texts: Some(scan.extracted.texts.clone()),
        images: None,
        tools: None,
        tool_calls,
        structured_messages: None,
        model: scan.model.clone(),
        stream_holdback_chars: None,
    };
    let pre = tool_call_shapes(&scan.extracted.tool_calls);
    let answer = guardrail
        .apply_guardrail(inputs, prepared, GuardrailInputType::Response, logging_obj)
        .await?;
    let post = post_shapes(&answer, &pre, context.guardrail_name)?;
    Ok(GuardrailAnswer {
        inputs: answer,
        pre,
        post,
    })
}

fn text_rewrites(
    scanned: &[String],
    rewritten: &[String],
    mappings: &[(usize, usize)],
) -> BTreeMap<(usize, usize), String> {
    let mut rewrites = BTreeMap::new();
    for ((scanned_text, mapping), rewritten_text) in scanned.iter().zip(mappings).zip(rewritten) {
        if scanned_text == rewritten_text {
            continue;
        }
        rewrites.insert(*mapping, rewritten_text.clone());
    }
    rewrites
}

fn deliver_terminal_rewrites(
    response: &mut Value,
    earlier: &mut [Value],
    mappings: &[(usize, usize)],
    scanned: &[String],
    rewritten: &[String],
    answer: &GuardrailAnswer,
    guardrail_name: Option<&str>,
) -> Result<(), StreamOutputError> {
    let rewrites = text_rewrites(scanned, rewritten, mappings);
    if !rewrites.is_empty() {
        stream_sync::sync_stream_events_with_rewrites(earlier, &rewrites);
    }
    let mut empty: [Value; 0] = [];
    let output = match response.get_mut("output") {
        Some(Value::Array(items)) => items.as_mut_slice(),
        Some(_) | None => empty.as_mut_slice(),
    };
    stream_sync::deliver_ended_stream_tool_call_rewrites(
        earlier,
        output,
        &answer.pre,
        &answer.post,
        guardrail_name.unwrap_or("unknown"),
    )
}

async fn terminal_scan<G>(
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
    let Some(response) = last.get_mut("response").filter(|value| value.is_object()) else {
        return Ok(false);
    };
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

async fn item_done_scan<G>(
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

fn response_model(events: &[Value]) -> Option<String> {
    events
        .last()
        .and_then(|last| last.get("response"))
        .and_then(Value::as_object)
        .and_then(|response| response.get("model"))
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

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

async fn fallback_scan<G>(
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
    let inputs = fallback_inputs(string_so_far.clone(), response_model(events));
    let mut fresh = Object::new();
    let prepared = request_data.unwrap_or(&mut fresh);
    let answer = guardrail
        .apply_guardrail(inputs, prepared, GuardrailInputType::Response, logging_obj)
        .await?;
    match changed_text(answer.texts.as_deref(), &string_so_far) {
        Some(rewritten) if context.deliver => {
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

async fn handle_last<G>(
    events: &mut [Value],
    guardrail: &G,
    context: &StreamContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<bool, StreamOutputError>
where
    G: GuardrailContract,
{
    let Some((last, earlier)) = events.split_last_mut() else {
        return Ok(true);
    };
    if !last.is_object() {
        return Ok(true);
    }
    let last_type = event_type(last).map(str::to_owned);
    match last_type.as_deref() {
        Some(kind) if TERMINAL_ENVELOPE_EVENT_TYPES.contains(&kind) => {
            terminal_scan(last, earlier, guardrail, context, logging_obj, request_data).await
        }
        Some("response.output_item.done") => {
            item_done_scan(last, guardrail, logging_obj, request_data).await?;
            Ok(!context.deliver)
        }
        Some(_) | None => Ok(false),
    }
}

// ast-grep-ignore: no-long-names
pub async fn process_output_streaming_response<G>(
    events: &mut [Value],
    guardrail: &G,
    guardrail_name: Option<&str>,
    mut logging_obj: Option<&mut GuardrailLogTarget>,
    user_api_key_dict: Option<&Object>,
    mut request_data: Option<&mut Object>,
    deliver_ended_stream_rewrites: bool,
) -> Result<(), StreamOutputError>
where
    G: GuardrailContract,
{
    let context = StreamContext {
        guardrail_name,
        user_api_key_dict,
        deliver: deliver_ended_stream_rewrites,
    };
    let handled = handle_last(
        events,
        guardrail,
        &context,
        logging_obj.as_deref_mut(),
        request_data.as_deref_mut(),
    )
    .await?;
    if handled {
        return Ok(());
    }
    fallback_scan(events, guardrail, &context, logging_obj, request_data).await
}
