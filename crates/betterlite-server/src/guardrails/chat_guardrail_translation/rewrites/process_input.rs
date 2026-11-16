use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::merge_messages::merge_scoped_messages;
use betterlite_guardrails::guardrail_message_scope::merge_tools::merge_returned_tools;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    ScanScope, ScopeFlags, scoped_message_indices,
};
use betterlite_guardrails::guardrail_message_scope::tool_names::openai_tool_name;

use super::apply_input_texts::apply_guardrailed_texts;
use super::apply_input_tool_calls::apply_guardrailed_tool_calls;
use super::error::{InputError, RewriteError};
use super::structured::{guardrail_messages, guardrail_tools, message_value, tool_value};
use crate::guardrails::chat_guardrail_translation::extraction::extract_inputs::extract_inputs;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::ExtractedContent;
use crate::guardrails::chat_guardrail_translation::extraction::json_access::object_items;
use crate::guardrails::chat_guardrail_translation::extraction::not_run_reason::not_run_reason;

mod tests;

pub struct InputScope<'scope> {
    pub flags: ScopeFlags,
    pub guardrail_name: Option<&'scope str>,
}

struct Scan<'scan> {
    scope: &'scan InputScope<'scan>,
    messages: Vec<GuardrailValue>,
    content: ExtractedContent,
}

fn request_inputs(
    data: &Object,
    scan: &Scan<'_>,
    scoped: &[usize],
) -> Result<GuardrailApiInputs, RewriteError> {
    let tools = object_items(data, "tools");
    let scan_only = scan.scope.flags.scan == ScanScope::ToolResultsOnly;
    let model = match data.get("model") {
        Some(GuardrailValue::Text(model)) if !model.is_empty() => Some(model.clone()),
        Some(_) | None => None,
    };
    Ok(GuardrailApiInputs {
        texts: Some(scan.content.texts.clone()),
        images: (!scan.content.images.is_empty()).then(|| scan.content.images.clone()),
        tools: if tools.is_empty() || scan_only {
            None
        } else {
            Some(guardrail_tools(tools)?)
        },
        tool_calls: (!scan.content.tool_calls.is_empty()).then(|| {
            scan.content
                .tool_calls
                .iter()
                .map(|call| GuardrailValue::Object(call.clone()))
                .collect()
        }),
        structured_messages: if scan.messages.is_empty() {
            None
        } else {
            Some(guardrail_messages(&scan.messages, scoped)?)
        },
        model,
        stream_holdback_chars: None,
    })
}

// Under a tool-result-only scan the guardrail's tools join the request's, so its answer never removes a tool.
fn write_returned_tools(data: &mut Object, returned: Vec<GuardrailValue>, scan_only: bool) {
    let merged = if scan_only {
        merge_returned_tools(object_items(data, "tools"), &returned, |tool| {
            openai_tool_name(tool).map(str::to_owned)
        })
    } else {
        returned
    };
    data.insert("tools".to_owned(), GuardrailValue::Array(merged));
}

// The guardrail's own texts and tool calls are written back only when it did not replace the whole message list.
fn write_returned_content(
    data: &mut Object,
    scan: &Scan<'_>,
    returned: GuardrailApiInputs,
) -> Result<(), InputError> {
    let Some(GuardrailValue::Array(messages)) = data.get_mut("messages") else {
        return Ok(());
    };
    let guardrailed_texts = returned.texts.unwrap_or_default();
    apply_guardrailed_texts(
        messages,
        &guardrailed_texts,
        &scan.content.texts,
        &scan.content.text_mappings,
        scan.scope.guardrail_name,
    )?;
    let tool_calls = returned.tool_calls.unwrap_or_default();
    apply_guardrailed_tool_calls(messages, tool_calls, &scan.content.tool_call_mappings)
}

async fn run_guardrail<G>(
    data: &mut Object,
    guardrail: &G,
    scan: &Scan<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), InputError>
where
    G: GuardrailContract,
{
    let scoped = scoped_message_indices(&scan.messages, &scan.scope.flags);
    let inputs = request_inputs(data, scan, &scoped)?;
    let sent_structured = inputs.structured_messages.clone();
    let mut returned = guardrail
        .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
        .await?;
    if let Some(tools) = returned.tools.take() {
        let scan_only = scan.scope.flags.scan == ScanScope::ToolResultsOnly;
        write_returned_tools(data, tools.into_iter().map(tool_value).collect(), scan_only);
    }
    let Some(back) = returned
        .structured_messages
        .take()
        .filter(|back| Some(back) != sent_structured.as_ref())
    else {
        return write_returned_content(data, scan, returned);
    };
    let replaced: Vec<GuardrailValue> = back.into_iter().map(message_value).collect();
    let messages = if guardrail.messages_cover_full_request() {
        replaced
    } else {
        merge_scoped_messages(&scan.messages, &scoped, &replaced)
    };
    data.insert("messages".to_owned(), GuardrailValue::Array(messages));
    Ok(())
}

// The Ok value is the reason nothing was scanned, for the caller to record when its guardrail keeps no record of its own.
pub async fn process_input_messages<G>(
    data: &mut Object,
    guardrail: &G,
    scope: &InputScope<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<Option<&'static str>, InputError>
where
    G: GuardrailContract,
{
    let messages = match data.get("messages") {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Array(messages)) => messages.clone(),
        Some(_) => {
            return Err(InputError::from(RewriteError::NotAList {
                field: "messages",
            }));
        }
    };
    let mut content = ExtractedContent::default();
    for (message_index, message) in messages.iter().enumerate() {
        extract_inputs(message, message_index, &scope.flags, &mut content);
    }
    if content.texts.is_empty() && content.tool_calls.is_empty() {
        return Ok(content
            .images
            .is_empty()
            .then(|| not_run_reason(&messages))
            .flatten());
    }
    let scan = Scan {
        scope,
        messages,
        content,
    };
    run_guardrail(data, guardrail, &scan, logging_obj).await?;
    Ok(None)
}
