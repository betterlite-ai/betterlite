use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::wire::parse_request_data;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::merge_user_key_metadata;

use super::error::RewriteError;
use super::principal::user_key_dict;
use super::response::{ChatResponse, ContentPart, MessageContent};
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::ExtractedContent;

mod tests;

// Only string content or a tool call makes a response worth scanning; list content alone is skipped.
#[must_use]
pub fn response_has_text(response: &ChatResponse) -> bool {
    response.choices.iter().any(|choice| {
        matches!(&choice.message.content, Some(MessageContent::Text(text)) if !text.is_empty())
            || choice
                .message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty())
    })
}

fn extract_parts(parts: &[ContentPart], choice_index: usize, out: &mut ExtractedContent) {
    for (content_index, part) in parts.iter().enumerate() {
        if let Some(text) = part.text.as_ref().filter(|text| !text.is_empty()) {
            out.texts.push(text.clone());
            out.text_mappings.push((choice_index, Some(content_index)));
        }
        let image_url = part
            .image_url
            .as_ref()
            .and_then(|image| image.url.as_ref())
            .filter(|url| !url.is_empty());
        if part.kind.as_deref() == Some("image_url")
            && let Some(url) = image_url
        {
            out.images.push(url.clone());
        }
    }
}

#[must_use]
pub fn extract_response(response: &ChatResponse) -> ExtractedContent {
    let mut out = ExtractedContent::default();
    for (choice_index, choice) in response.choices.iter().enumerate() {
        match &choice.message.content {
            Some(MessageContent::Text(text)) if !text.is_empty() => {
                out.texts.push(text.clone());
                out.text_mappings.push((choice_index, None));
            }
            Some(MessageContent::Parts(parts)) => extract_parts(parts, choice_index, &mut out),
            Some(MessageContent::Text(_)) | None => {}
        }
        let calls = choice.message.tool_calls.iter().flatten();
        for (tool_call_index, call) in calls.enumerate() {
            if let Some(object) = call.to_object() {
                out.tool_calls.push(object);
                out.tool_call_mappings.push((choice_index, tool_call_index));
            }
        }
    }
    out
}

// An empty images or tool_calls list is left out, as LiteLLM only sets those keys when it found something.
#[must_use]
pub fn output_inputs(response: &ChatResponse, extracted: &ExtractedContent) -> GuardrailApiInputs {
    let model = response.model.clone().filter(|model| !model.is_empty());
    let images = (!extracted.images.is_empty()).then(|| extracted.images.clone());
    let tool_calls = (!extracted.tool_calls.is_empty()).then(|| {
        extracted
            .tool_calls
            .iter()
            .map(|call| GuardrailValue::Object(call.clone()))
            .collect()
    });
    GuardrailApiInputs {
        texts: Some(extracted.texts.clone()),
        images,
        tools: None,
        tool_calls,
        structured_messages: None,
        model,
        stream_holdback_chars: None,
    }
}

// A fresh dict holding the response and the key's metadata, as LiteLLM builds it when the caller passes no request data.
pub fn output_request_data(
    response: &ChatResponse,
    principal: &Principal,
) -> Result<Object, RewriteError> {
    let wire = parse_request_data(&serde_json::to_string(response)?)?;
    let mut request_data = Object::new();
    request_data.insert("response".to_owned(), GuardrailValue::Object(wire));
    merge_user_key_metadata(&mut request_data, Some(&user_key_dict(principal)));
    Ok(request_data)
}
