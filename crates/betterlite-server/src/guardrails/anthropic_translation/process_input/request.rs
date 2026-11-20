use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    ScanScope, ScopeFlags, scoped_message_indices,
};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use serde_json::{Map, Value};

use crate::guardrails::anthropic_translation::input_scan::error::ConvertError;
use crate::guardrails::anthropic_translation::input_scan::{
    extract_input_text_and_images, extract_top_level_system_text, serde_convert,
};
use crate::guardrails::anthropic_translation::structured_write_back::{
    hoisted_system_message, translate_to_openai,
};
use crate::guardrails::tool_call_identity::tool_call_shapes;

use super::error::ProcessInputError;
use super::{
    InputContext, MESSAGES, ROLE, RequestScan, RequestState, SYSTEM, StructuredView, TOOLS,
};

struct ChatView {
    tools: Vec<PromptObject>,
    messages: Vec<Value>,
}

pub(super) async fn prepare_request(
    data: &Object,
    original_messages: &[Value],
    context: &InputContext<'_>,
) -> Result<RequestState, ProcessInputError> {
    let flags = ScopeFlags::resolve(&context.skip_flags, &context.global_flags);
    let scan_only = flags.scan == ScanScope::ToolResultsOnly;
    let body: Map<String, Value> = serde_convert(data, "request body")?;
    let ChatView { tools, messages } = chat_view(&body).await?;
    let mut full = messages;
    let has_midturn_system_message = full.iter().any(is_system_row);
    let hoisted = if flags.skip_system_message {
        None
    } else {
        hoisted_system_message(&body).await?
    };
    if let Some(message) = &hoisted {
        full.insert(0, message.clone());
    }
    let (indices, sent) = scoped_rows(&full, flags)?;
    let scanned = scan_messages(original_messages, hoisted.as_ref(), scan_only, flags);
    let structured = StructuredView {
        full,
        indices,
        sent,
        hoisted,
        has_midturn_system_message,
    };
    Ok(RequestState {
        tools,
        structured,
        scanned,
        scan_only,
    })
}

async fn chat_view(body: &Map<String, Value>) -> Result<ChatView, ProcessInputError> {
    let chat = translate_to_openai(&without_system(body)).await?;
    let tools: Vec<PromptObject> = match chat.get(TOOLS) {
        Some(PromptJson::List(items)) => items
            .iter()
            .filter_map(PromptJson::as_object)
            .cloned()
            .collect(),
        Some(_) | None => Vec::new(),
    };
    let chat_value = serde_json::to_value(&chat).map_err(|source| ConvertError::Convert {
        target: "chat request",
        source,
    })?;
    let messages = chat_value
        .get(MESSAGES)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(ChatView { tools, messages })
}

fn scoped_rows(
    full: &[Value],
    flags: ScopeFlags,
) -> Result<(Vec<usize>, Vec<GuardrailMessage>), ProcessInputError> {
    // The trusted top-level prompt is already out of the list when it was skipped, so every row stays in scope.
    let scope_flags = ScopeFlags {
        skip_system_message: false,
        ..flags
    };
    let full_guardrail: Vec<GuardrailValue> = serde_convert(full, "structured messages")?;
    let indices = scoped_message_indices(&full_guardrail, &scope_flags);
    let mut sent = Vec::with_capacity(indices.len());
    for index in &indices {
        let Some(row) = full.get(*index) else {
            continue;
        };
        sent.push(serde_convert::<_, GuardrailMessage>(
            row,
            "structured message",
        )?);
    }
    Ok((indices, sent))
}

fn scan_messages(
    original_messages: &[Value],
    hoisted: Option<&Value>,
    scan_only: bool,
    flags: ScopeFlags,
) -> RequestScan {
    let mut scanned = RequestScan::default();
    if let Some(message) = hoisted
        && !scan_only
    {
        scanned.scanned = extract_top_level_system_text(message);
    }
    for (msg_idx, original) in original_messages.iter().enumerate() {
        let extracted = extract_input_text_and_images(original, msg_idx, flags);
        scanned.scanned.extend(extracted.scanned);
        scanned.images.extend(extracted.images);
        scanned.scanned_tool_calls.extend(extracted.tool_calls);
    }
    scanned.texts = scanned
        .scanned
        .iter()
        .map(|item| item.text.clone())
        .collect();
    scanned.tool_calls = scanned
        .scanned_tool_calls
        .iter()
        .map(|call| call.tool_call.clone())
        .collect();
    scanned.pre_guardrail_tool_calls = tool_call_shapes(&scanned.tool_calls);
    scanned
}

fn is_system_row(message: &Value) -> bool {
    message
        .get(ROLE)
        .and_then(Value::as_str)
        .is_some_and(|role| role.eq_ignore_ascii_case(SYSTEM))
}

fn without_system(body: &Map<String, Value>) -> Map<String, Value> {
    let mut source = body.clone();
    source.remove(SYSTEM);
    source
}
