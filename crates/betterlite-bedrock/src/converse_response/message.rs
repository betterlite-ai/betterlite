use betterlite_models::call_helpers::finish_reason::{FinishReason, map_finish_reason};
use betterlite_models::decisions::{Extra, Json};
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::agentcore_stream::IdSource;
use crate::converse_content::citations::{CitationAnnotation, citations_to_annotations};
use crate::converse_content::json_mode::filter_json_mode_tools;
use crate::converse_content::recovery::{AssistantMessage, recover_tool_call};
use crate::converse_content::translation::{Translation, translate_message_content};
use crate::converse_content::{ContentBlock, ConverseContentError, ToolCall};
use crate::converse_response::request::ReplyRequest;
use crate::converse_response::wire::{Message, ReplyToolCall};
use crate::converse_response::{ConverseReply, ReplyFieldError};
use crate::converse_usage::error::ConverseUsageError;
use crate::converse_usage::reasoning::{
    ThinkingBlock, transform_reasoning_content, transform_thinking_blocks,
};

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error(transparent)]
    Reply(#[from] ReplyFieldError),
    #[error(transparent)]
    Content(#[from] ConverseContentError),
    #[error(transparent)]
    Reasoning(#[from] ConverseUsageError),
}

pub struct ConvertedMessage {
    pub message: Message,
    pub finish_reason: String,
}

pub fn convert_message(
    reply: &ConverseReply,
    request: &ReplyRequest<'_>,
    ids: &dyn IdSource,
) -> Result<ConvertedMessage, MessageError> {
    let json_mode = request
        .json_mode
        .or(request.optional.json_mode)
        .unwrap_or(false);
    let translation = translate(reply, request)?;
    let (content, annotations) = align_citations(&translation);
    let filtered = filter_json_mode_tools(json_mode, &translation.tools, Some(content.as_str()));
    let answer = filtered.content.unwrap_or(content);
    let content_tools = filtered.tools.as_deref();
    let finish_reason = forced_finish_reason(reply, json_mode, content_tools, &translation.tools)?;
    let (recovered, resolved_finish_reason) =
        with_recovery(answer, content_tools, request, finish_reason, ids);
    let (reasoning_content, thinking_blocks) =
        reasoning_blocks(translation.reasoning_content_blocks.as_deref())?;
    Ok(ConvertedMessage {
        message: Message {
            content: recovered.content,
            role: "assistant".to_owned(),
            tool_calls: recovered.tool_calls,
            function_call: None,
            provider_specific_fields: provider_fields(&translation),
            annotations,
            reasoning_content,
            thinking_blocks,
        },
        finish_reason: resolved_finish_reason,
    })
}

// LiteLLM hands ConverseResponseBlock's output.message straight to the content translation; null is the legal empty message.
fn translate(
    reply: &ConverseReply,
    request: &ReplyRequest<'_>,
) -> Result<Translation, MessageError> {
    let Some(message) = reply.message()? else {
        return Ok(Translation {
            content: String::new(),
            tools: Vec::new(),
            reasoning_content_blocks: None,
            citations_content_blocks: None,
        });
    };
    let blocks = content_blocks(message)?;
    Ok(translate_message_content(
        &blocks,
        request.tool_name_mappings,
    )?)
}

fn content_blocks(message: &Json) -> Result<Vec<ContentBlock>, ReplyFieldError> {
    let Json::Object(fields) = message else {
        return Err(ReplyFieldError::NotA {
            path: "output.message",
            expected: "an object",
        });
    };
    let Some(content) = fields.get("content") else {
        return Err(ReplyFieldError::Missing {
            path: "output.message.content",
        });
    };
    decode_blocks(content)
}

// The block list round-trips through its JSON text, the way ConverseResponseBlock is rebuilt from the parsed body.
fn decode_blocks(content: &Json) -> Result<Vec<ContentBlock>, ReplyFieldError> {
    let Ok(text) = serde_json::to_string(content) else {
        return Err(not_a_block_list());
    };
    serde_json::from_str(&text).map_err(|_error| not_a_block_list())
}

const fn not_a_block_list() -> ReplyFieldError {
    ReplyFieldError::NotA {
        path: "output.message.content",
        expected: "a list of content blocks",
    }
}

// A required stopReason maps through B206; a null or non-string scalar is simply unmapped, while a list or object is unhashable in Python's map lookup.
fn forced_finish_reason(
    reply: &ConverseReply,
    json_mode: bool,
    filtered: Option<&[ToolCall]>,
    original: &[ToolCall],
) -> Result<&'static str, MessageError> {
    let mapped = match reply.stop_reason()? {
        Json::Text(reason) => map_finish_reason(reason).reason,
        Json::Null | Json::Bool(_) | Json::Number(_) => FinishReason::Stop,
        Json::List(_) | Json::Object(_) => {
            return Err(MessageError::Reply(ReplyFieldError::NotA {
                path: "stopReason",
                expected: "a string",
            }));
        }
    };
    // json_mode turned every synthetic call into content, so the reply is a plain answer rather than a tool invocation.
    if json_mode && filtered.is_none_or(<[ToolCall]>::is_empty) && !original.is_empty() {
        return Ok(FinishReason::Stop.as_str());
    }
    Ok(mapped.as_str())
}

struct Recovered {
    content: Option<String>,
    tool_calls: Option<Vec<ReplyToolCall>>,
}

// A textual function call replaces the parsed tool calls and blanks the content, exactly as the recovery helper mutates the message.
fn with_recovery(
    content: String,
    content_tools: Option<&[ToolCall]>,
    request: &ReplyRequest<'_>,
    finish_reason: &'static str,
    ids: &dyn IdSource,
) -> (Recovered, String) {
    let mut message = AssistantMessage {
        content: Some(content),
        tool_calls: None,
    };
    let recovered = recover_tool_call(
        &mut message,
        request.optional.tools.as_deref(),
        Some(finish_reason),
        &|| ids.next_id(),
    );
    let resolved_finish_reason = recovered.unwrap_or_else(|| finish_reason.to_owned());
    let tool_calls: Option<Vec<ReplyToolCall>> = message.tool_calls.as_deref().map_or_else(
        || content_tools.map(|tools| tools.iter().map(ReplyToolCall::from).collect()),
        |calls| Some(calls.iter().map(ReplyToolCall::from).collect()),
    );
    (
        Recovered {
            content: message.content,
            tool_calls,
        },
        resolved_finish_reason,
    )
}

// LiteLLM keeps both raw block lists under provider_specific_fields whenever the lists are not None.
fn provider_fields(translation: &Translation) -> Option<Extra> {
    let mut fields = Extra::new();
    if let Some(blocks) = &translation.reasoning_content_blocks {
        fields.insert(
            "reasoningContentBlocks".to_owned(),
            Json::List(blocks.clone()),
        );
    }
    if let Some(blocks) = &translation.citations_content_blocks {
        fields.insert("citationsContent".to_owned(), Json::List(blocks.clone()));
    }
    (!fields.is_empty()).then_some(fields)
}

fn reasoning_blocks(
    blocks: Option<&[Json]>,
) -> Result<(Option<String>, Option<Vec<ThinkingBlock>>), ConverseUsageError> {
    let Some(present) = blocks else {
        return Ok((None, None));
    };
    let content = transform_reasoning_content(present)?;
    let thinking = transform_thinking_blocks(present)?;
    Ok((Some(content), Some(thinking)))
}

// The cited text replaces whitespace-only content or is stitched in front of punctuation-only content, so the annotation spans line up.
fn align_citations(translation: &Translation) -> (String, Option<Vec<CitationAnnotation>>) {
    let blocks = citations_to_annotations(translation.citations_content_blocks.as_deref());
    let content = translation.content.clone();
    let Some(text) = blocks.text.as_deref().filter(|text| !text.is_empty()) else {
        return (content, None);
    };
    let stripped = content.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    });
    if stripped.is_empty() {
        return (text.to_owned(), blocks.annotations);
    }
    if stripped.chars().any(is_alphanumeric) {
        return (content, None);
    }
    (format!("{text}{content}"), blocks.annotations)
}

// Python's str.isalnum(): the Lu/Ll/Lt/Lm/Lo letter categories or the Nd/Nl/No number categories.
fn is_alphanumeric(character: char) -> bool {
    matches!(
        get_general_category(character),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::DecimalNumber
            | GeneralCategory::LetterNumber
            | GeneralCategory::OtherNumber
    )
}
