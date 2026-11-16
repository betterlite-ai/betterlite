#![expect(
    clippy::disallowed_types,
    reason = "Responses output items are provider JSON that betterlite does not model"
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

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_chat_history::tool_fields::output_tool_fields;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::input_provenance::{is_tool_call_output_item, tool_call_output_item_mapping};
use crate::guardrails::response_context::add_response_context;
use crate::guardrails::tool_call_identity::{
    ToolCallShape, post_guardrail_shapes, tool_call_rewrite, tool_call_shapes,
};
use error::OutputRewriteError;

pub struct OutputContext<'ctx> {
    pub guardrail_name: Option<&'ctx str>,
    pub user_api_key_dict: Option<&'ctx Object>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedOutput {
    pub texts: Vec<String>,
    pub task_mappings: Vec<(usize, usize)>,
    pub tool_calls: Vec<Value>,
}

// The one place this module moves between JSON values and the typed shapes around it
fn serde_convert<Source, Target>(
    value: &Source,
    target: &'static str,
) -> Result<Target, OutputRewriteError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|source| OutputRewriteError::Convert { target, source })?;
    serde_json::from_value(json).map_err(|source| OutputRewriteError::Convert { target, source })
}

pub fn extract_output(output_items: &[Value]) -> Result<ExtractedOutput, OutputRewriteError> {
    let mut extracted = ExtractedOutput {
        texts: Vec::new(),
        task_mappings: Vec::new(),
        tool_calls: Vec::new(),
    };
    for (output_index, item) in output_items.iter().enumerate() {
        if let Some(call) = tool_call_output_item_mapping(item) {
            let chunk = output_tool_fields(call, output_index)?;
            extracted
                .tool_calls
                .push(serde_convert(&chunk, "tool call")?);
            continue;
        }
        let Some(content) = item
            .get("content")
            .and_then(Value::as_array)
            .filter(|content| !content.is_empty())
        else {
            continue;
        };
        tracing::debug!("OpenAI Responses API: Processing output item: {item}");
        for (content_index, entry) in content.iter().enumerate() {
            if let Some(Value::String(text)) = entry.get("text")
                && !text.is_empty()
            {
                extracted.texts.push(text.clone());
                extracted.task_mappings.push((output_index, content_index));
            }
        }
    }
    Ok(extracted)
}

pub fn apply_texts_to_output(
    response: &mut Value,
    responses: &[String],
    task_mappings: &[(usize, usize)],
) -> Result<(), OutputRewriteError> {
    let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for (index, text) in responses.iter().enumerate() {
        let Some((output_index, content_index)) = task_mappings.get(index) else {
            return Err(OutputRewriteError::TooManyTexts {
                index,
                scanned: task_mappings.len(),
            });
        };
        let slot = output
            .get_mut(*output_index)
            .and_then(|item| item.get_mut("content"))
            .and_then(Value::as_array_mut)
            .and_then(|content| content.get_mut(*content_index))
            .and_then(Value::as_object_mut);
        if let Some(entry) = slot {
            entry.insert("text".to_owned(), Value::String(text.clone()));
        }
    }
    Ok(())
}

// A missing payload leaves arguments and input alone, which the stream's output_item.added events rely on
pub fn write_tool_call_item(item: &mut Value, new_name: Option<&str>, new_payload: Option<&str>) {
    let Some(fields) = item.as_object_mut() else {
        return;
    };
    if let Some(name) = new_name {
        fields.insert("name".to_owned(), Value::String(name.to_owned()));
    }
    let Some(payload) = new_payload else {
        return;
    };
    let field = match fields.get("type").and_then(Value::as_str) {
        Some("function_call") => "arguments",
        Some("custom_tool_call") => "input",
        Some(_) | None => return,
    };
    fields.insert(field.to_owned(), Value::String(payload.to_owned()));
}

// A guardrail that adds or drops a call is ignored, so the originals stay
pub fn write_call_rewrites(
    response: &mut Value,
    pre_guardrail: &[ToolCallShape],
    post_guardrail: &[ToolCallShape],
) {
    let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };
    let calls: Vec<&mut Value> = output
        .iter_mut()
        .filter(|item| is_tool_call_output_item(item))
        .collect();
    if calls.len() != post_guardrail.len() {
        return;
    }
    for ((item, before), after) in calls.into_iter().zip(pre_guardrail).zip(post_guardrail) {
        if before != after {
            let rewrite = tool_call_rewrite(before, after);
            write_tool_call_item(item, rewrite.name.as_deref(), Some(&rewrite.arguments));
        }
    }
}

fn scanned_output(response: &Value) -> Result<Option<ExtractedOutput>, OutputRewriteError> {
    let Some(fields) = response.as_object() else {
        tracing::debug!("OpenAI Responses API: No output found in response");
        return Ok(None);
    };
    let items: &[Value] = match fields.get("output") {
        None | Some(Value::Null) => &[],
        Some(Value::Array(items)) => items.as_slice(),
        Some(_) => return Err(OutputRewriteError::FieldType { field: "output" }),
    };
    if items.is_empty() {
        tracing::debug!("OpenAI Responses API: Empty output in response");
        return Ok(None);
    }
    Ok(Some(extract_output(items)?))
}

fn scan_inputs(
    response: &Value,
    extracted: &ExtractedOutput,
) -> Result<GuardrailApiInputs, OutputRewriteError> {
    let tool_calls = if extracted.tool_calls.is_empty() {
        None
    } else {
        Some(serde_convert(&extracted.tool_calls, "tool calls")?)
    };
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(str::to_owned);
    Ok(GuardrailApiInputs {
        texts: Some(extracted.texts.clone()),
        images: None,
        tools: None,
        tool_calls,
        structured_messages: None,
        model,
        stream_holdback_chars: None,
    })
}

// Texts are written first, and a text with no mapping stops the tool call rewrites like LiteLLM's IndexError does
fn write_answer(
    response: &mut Value,
    extracted: &ExtractedOutput,
    answer: &GuardrailApiInputs,
    guardrail_name: Option<&str>,
) -> Result<(), OutputRewriteError> {
    let pre_guardrail = tool_call_shapes(&extracted.tool_calls);
    let returned_calls: Option<Vec<Value>> = answer
        .tool_calls
        .as_ref()
        .map(|calls| serde_convert(calls, "returned tool calls"))
        .transpose()?;
    let post_guardrail =
        post_guardrail_shapes(returned_calls.as_deref(), &pre_guardrail, guardrail_name);
    let texts = answer.texts.as_deref().unwrap_or_default();
    apply_texts_to_output(response, texts, &extracted.task_mappings)?;
    write_call_rewrites(response, &pre_guardrail, &post_guardrail);
    Ok(())
}

pub async fn process_output_response<G>(
    response: &mut GuardrailValue,
    guardrail: &G,
    context: &OutputContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<(), OutputRewriteError>
where
    G: GuardrailContract,
{
    let mut value: Value = serde_convert(&*response, "response")?;
    let Some(extracted) = scanned_output(&value)? else {
        return Ok(());
    };
    if extracted.texts.is_empty() && extracted.tool_calls.is_empty() {
        tracing::debug!("OpenAI Responses API: Processed output response: {response:?}");
        return Ok(());
    }
    let mut fresh_request_data = Object::new();
    let prepared_request_data = request_data.unwrap_or(&mut fresh_request_data);
    add_response_context(prepared_request_data, response, context.user_api_key_dict);
    let inputs = scan_inputs(&value, &extracted)?;
    let answer = guardrail
        .apply_guardrail(
            inputs,
            prepared_request_data,
            GuardrailInputType::Response,
            logging_obj,
        )
        .await
        .map_err(|source| OutputRewriteError::Guardrail {
            source: Box::new(source),
        })?;
    let written = write_answer(&mut value, &extracted, &answer, context.guardrail_name);
    *response = serde_convert(&value, "response")?;
    tracing::debug!("OpenAI Responses API: Processed output response: {response:?}");
    written
}
