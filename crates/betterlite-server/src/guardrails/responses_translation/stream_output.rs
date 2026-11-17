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

mod cases;
pub mod error;
mod fallback;
mod tests;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_chat_history::tool_fields::output_tool_fields;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use super::output_rewrite::error::OutputRewriteError;
use super::output_rewrite::{ExtractedOutput, extract_output};
use super::stream_scan::TERMINAL_ENVELOPE_EVENT_TYPES;
use super::tool_call_identity::{ToolCallShape, post_guardrail_shapes, tool_call_shapes};
use crate::guardrails::response_context::add_response_context;
use cases::{item_done_scan, terminal_scan};
use error::StreamOutputError;
use fallback::fallback_scan;

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
    let value: GuardrailValue = convert(response, "response")?;
    Ok(EnvelopeScan {
        extracted: extract_output(outputs)?,
        model: model_of(response),
        response: value,
    })
}

fn convert<Source, Target>(value: Source, target: &'static str) -> Result<Target, StreamOutputError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let value = serde_json::to_value(value)
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
    let returned: Vec<Value> = convert(calls, "returned tool calls")?;
    Ok(post_guardrail_shapes(Some(&returned), pre, guardrail_name))
}

fn tool_call_inputs(
    item: &Map<String, Value>,
    index: usize,
) -> Result<GuardrailApiInputs, StreamOutputError> {
    let chunk = output_tool_fields(item, index).map_err(OutputRewriteError::from)?;
    let tool_call: GuardrailValue = convert(&chunk, "tool call")?;
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
    let tool_calls: Option<Vec<GuardrailValue>> = if scan.extracted.tool_calls.is_empty() {
        None
    } else {
        Some(convert(&scan.extracted.tool_calls, "tool calls")?)
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
    let last_type = last.get("type").and_then(Value::as_str).map(str::to_owned);
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
