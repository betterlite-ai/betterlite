use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::merge_messages::merge_scoped_messages;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;
use serde_json::{Map, Value};

use crate::guardrails::anthropic_translation::input_scan::serde_convert;
use crate::guardrails::anthropic_translation::scan_targets::{ScannedText, ScannedToolCall};
use crate::guardrails::anthropic_translation::structured_write_back::write_back_structured_messages;
use crate::guardrails::anthropic_translation::write_back::{
    apply_responses_to_input, apply_tool_calls_to_input,
};
use crate::guardrails::tool_call_identity::ToolCallShape;

use super::error::ProcessInputError;
use super::tools::write_returned_tools;
use super::{AnswerContext, MESSAGES};

struct TextAnswer<'answer> {
    texts: &'answer [String],
    tool_calls: Option<&'answer [GuardrailValue]>,
    scanned: &'answer [ScannedText],
    scanned_tool_calls: &'answer [ScannedToolCall],
    pre_guardrail_tool_calls: &'answer [ToolCallShape],
    guardrail_name: Option<&'answer str>,
}

pub(super) async fn run_guardrail<G>(
    data: &mut Object,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    inputs: GuardrailApiInputs,
    context: &AnswerContext<'_>,
) -> Result<(), ProcessInputError>
where
    G: GuardrailContract,
{
    let answer = guardrail
        .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
        .await
        .map_err(|source| ProcessInputError::Guardrail {
            source: Box::new(source),
        })?;
    write_back(data, &answer, context).await
}

async fn write_back(
    data: &mut Object,
    answer: &GuardrailApiInputs,
    context: &AnswerContext<'_>,
) -> Result<(), ProcessInputError> {
    if let Some(returned) = &answer.tools {
        write_returned_tools(data, returned, context.scan_only)?;
    }
    let mut written_body: Map<String, Value> = serde_convert(data, "request data")?;
    write_answer(&mut written_body, answer, context).await?;
    *data = serde_convert(&written_body, "request body")?;
    Ok(())
}

async fn write_answer(
    body: &mut Map<String, Value>,
    answer: &GuardrailApiInputs,
    context: &AnswerContext<'_>,
) -> Result<(), ProcessInputError> {
    let Some(returned) = returned_structured(answer, &context.structured.sent) else {
        return apply_text_answer(body, &text_answer(answer, context));
    };
    let rows = structured_rows(
        returned,
        &context.structured.full,
        &context.structured.indices,
        context.covers_full_request,
    )?;
    write_back_structured_messages(
        body,
        &rows,
        context.structured.hoisted.as_ref(),
        context.structured.has_midturn_system_message,
    )
    .await?;
    Ok(())
}

fn text_answer<'answer>(
    answer: &'answer GuardrailApiInputs,
    context: &'answer AnswerContext<'_>,
) -> TextAnswer<'answer> {
    TextAnswer {
        texts: answer.texts.as_deref().unwrap_or_default(),
        tool_calls: answer.tool_calls.as_deref(),
        scanned: &context.scanned.scanned,
        scanned_tool_calls: &context.scanned.scanned_tool_calls,
        pre_guardrail_tool_calls: &context.scanned.pre_guardrail_tool_calls,
        guardrail_name: context.guardrail_name,
    }
}

fn apply_text_answer(
    body: &mut Map<String, Value>,
    answer: &TextAnswer<'_>,
) -> Result<(), ProcessInputError> {
    if !answer.texts.is_empty() && answer.texts.len() != answer.scanned.len() {
        return Err(ProcessInputError::from(UnappliableRequestRewrite::new(
            answer.guardrail_name,
        )));
    }
    let returned_tool_calls: Option<Vec<Value>> = answer
        .tool_calls
        .map(|calls| serde_convert(calls, "tool calls"))
        .transpose()?;
    let Some(Value::Array(messages)) = body.get_mut(MESSAGES) else {
        return Err(ProcessInputError::FieldType {
            field: MESSAGES,
            expected: "an array",
        });
    };
    apply_tool_calls_to_input(
        messages,
        answer.scanned_tool_calls,
        answer.pre_guardrail_tool_calls,
        returned_tool_calls.as_deref(),
        answer.guardrail_name,
    )?;
    apply_responses_to_input(body, answer.texts, answer.scanned)?;
    Ok(())
}

fn returned_structured<'answer>(
    answer: &'answer GuardrailApiInputs,
    sent: &[GuardrailMessage],
) -> Option<&'answer [GuardrailMessage]> {
    answer
        .structured_messages
        .as_deref()
        .filter(|returned| *returned != sent)
}

fn structured_rows(
    returned: &[GuardrailMessage],
    full_messages: &[Value],
    scoped_indices: &[usize],
    covers_full_request: bool,
) -> Result<Vec<Value>, ProcessInputError> {
    let mut returned_values = Vec::with_capacity(returned.len());
    for row in returned {
        returned_values.push(serde_convert::<_, Value>(row, "structured message")?);
    }
    if covers_full_request {
        return Ok(returned_values);
    }
    Ok(merge_scoped_messages(
        full_messages,
        scoped_indices,
        &returned_values,
    ))
}
