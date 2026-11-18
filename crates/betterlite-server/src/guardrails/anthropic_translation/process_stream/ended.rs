use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_streaming::streamed_response_assembly::response::ChatMessage;
use serde_json::Value;

use super::StreamContext;
use super::assembly::{assemble_stream_response, scanned_chat};
use super::convert::convert;
use super::error::{ProcessStreamError, keep_original_response};
use super::prepare::prepare_request_data;
use crate::guardrails::anthropic_translation::sse_events::SseChunk;
use crate::guardrails::anthropic_translation::stream_rewrite::ended;
use crate::guardrails::tool_call_identity::tool_call_shapes;

// LiteLLM sends texts only for non-empty content and tool calls only when the response carried some.
fn scan_inputs(
    message: &ChatMessage,
    pre: &[Value],
) -> Result<GuardrailApiInputs, ProcessStreamError> {
    let texts = message
        .content
        .as_ref()
        .filter(|text| !text.is_empty())
        .map(|text| vec![text.clone()]);
    let tool_calls = (!pre.is_empty()).then(|| convert(pre)).transpose()?;
    Ok(GuardrailApiInputs {
        texts,
        tool_calls,
        ..GuardrailApiInputs::default()
    })
}

// The text rewrite needs a non-empty original content, a returned text and a change.
fn write_text(
    responses_so_far: &mut [SseChunk],
    answer: &GuardrailApiInputs,
    content: Option<&str>,
    guardrail_name: &str,
) -> Result<(), ProcessStreamError> {
    let Some(original) = content.filter(|text| !text.is_empty()) else {
        return Ok(());
    };
    let Some(rewritten) = answer.texts.as_ref().and_then(|texts| texts.first()) else {
        return Ok(());
    };
    if rewritten.as_str() == original {
        return Ok(());
    }
    // ast-grep-ignore: no-long-names
    ended::write_ended_stream_text_rewrite(responses_so_far, rewritten, guardrail_name)?;
    Ok(())
}

// A returned tool call list of another length is ignored, so the original shapes come back and nothing changes.
fn write_tools(
    responses_so_far: &mut [SseChunk],
    answer: &GuardrailApiInputs,
    pre: &[Value],
    guardrail_name: &str,
) -> Result<(), ProcessStreamError> {
    let pre_shapes = tool_call_shapes(pre);
    let post_values = match answer.tool_calls.as_deref() {
        Some(returned) if returned.len() == pre.len() => convert(returned)?,
        Some(_) | None => pre.to_vec(),
    };
    let post_shapes = tool_call_shapes(&post_values);
    // ast-grep-ignore: no-long-names
    ended::write_ended_stream_tool_call_rewrites(
        responses_so_far,
        &pre_shapes,
        &post_shapes,
        guardrail_name,
    )?;
    Ok(())
}

pub(super) async fn process_ended<G>(
    responses_so_far: &mut [SseChunk],
    guardrail: &G,
    context: &StreamContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    mut request_data: Option<&mut Object>,
) -> Result<(), ProcessStreamError>
where
    G: GuardrailContract,
{
    let assembled = assemble_stream_response(responses_so_far).await?;
    let Some((chat, message)) = scanned_chat(assembled.as_ref()) else {
        tracing::debug!("Skipping output guardrail - model response has no choices");
        return Ok(());
    };
    let pre = match message.tool_calls.as_deref() {
        Some(calls) => convert(calls)?,
        None => Vec::new(),
    };
    let inputs = scan_inputs(message, &pre)?;
    let content = message.content.clone();
    let response_value = convert(chat)?;
    let mut fresh = Object::new();
    let prepared = request_data.as_deref_mut().unwrap_or(&mut fresh);
    prepare_request_data(
        prepared,
        response_value.clone(),
        context.user_api_key_dict,
        "response",
    );
    let answer = guardrail
        .apply_guardrail(inputs, prepared, GuardrailInputType::Response, logging_obj)
        .await
        .map_err(|failure| keep_original_response(failure, Some(response_value)))?;
    if !context.deliver {
        return Ok(());
    }
    write_text(
        responses_so_far,
        &answer,
        content.as_deref(),
        context.guardrail_name,
    )?;
    write_tools(responses_so_far, &answer, &pre, context.guardrail_name)
}
