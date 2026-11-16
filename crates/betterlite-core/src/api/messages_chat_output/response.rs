use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;

use crate::api::messages_chat_output::content::{anthropic_finish_reason, content_blocks};
use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::output::{
    AnthropicStopReason, AssistantRole, ContentEntry, ContextManagement, MessageKind,
    MessageResponse, UsageProjection,
};
use crate::api::messages_chat_output::records::{ResponseRecord, UsageRecord, refusal_text};
use crate::api::messages_chat_output::usage::project_usage;
use crate::api::messages_context_clear::result::{
    CompactionBlock, PolyfillResult, UsageIteration, UsageKind,
};
use crate::api::messages_options::refusal::refusal_stop_details;

pub const UNKNOWN_MODEL: &str = "unknown-model";

pub fn response_message(
    response: &ResponseRecord,
    tool_name_mapping: Option<&IndexMap<String, String>>,
    polyfill_result: Option<&PolyfillResult>,
) -> Result<MessageResponse, ChatOutputError> {
    let mut content = content_blocks(&response.choices, tool_name_mapping)?;
    let refusal = response.choices.iter().find_map(|choice| {
        refusal_text(
            &choice.message.refusal,
            &choice.message.provider_specific_fields,
        )
    });
    if let Some(block) = polyfill_result.and_then(|result| result.compaction_block.as_ref()) {
        content.insert(0, ContentEntry::Compaction(compaction_object(block)));
    }
    let openai_reason = response
        .choices
        .first()
        .and_then(|choice| choice.finish_reason.as_deref())
        .unwrap_or("stop");
    let translated = anthropic_finish_reason(openai_reason);
    let stop_reason = stop_reason(&content, refusal.as_deref(), translated);
    Ok(MessageResponse {
        id: response.id.clone(),
        kind: MessageKind::Message,
        role: AssistantRole::Assistant,
        model: if response.model.is_empty() {
            UNKNOWN_MODEL.to_owned()
        } else {
            response.model.clone()
        },
        stop_sequence: None,
        usage: response_usage(response, polyfill_result),
        content,
        stop_reason,
        stop_details: (stop_reason == AnthropicStopReason::Refusal)
            .then(|| refusal_stop_details(refusal)),
        context_management: polyfill_result
            .and_then(PolyfillResult::applied_edits_for_response)
            .map(|applied_edits| ContextManagement { applied_edits }),
    })
}

// One compaction block alone wins over every stop reason; a refusal forces refusal unless truncation was cut.
fn stop_reason(
    content: &[ContentEntry],
    refusal_text: Option<&str>,
    translated: AnthropicStopReason,
) -> AnthropicStopReason {
    let only_compaction =
        content.len() == 1 && matches!(content.first(), Some(ContentEntry::Compaction(_)));
    if only_compaction {
        return AnthropicStopReason::Compaction;
    }
    let refused = refusal_text.is_some() && translated != AnthropicStopReason::MaxTokens;
    if refused {
        return AnthropicStopReason::Refusal;
    }
    translated
}

fn response_usage(
    response: &ResponseRecord,
    polyfill_result: Option<&PolyfillResult>,
) -> UsageProjection {
    let mut usage = response
        .usage
        .as_ref()
        .map_or_else(|| project_usage(&UsageRecord::default()), project_usage);
    let Some(iterations) = polyfill_result.and_then(|result| result.iterations_usage.as_deref())
    else {
        return usage;
    };
    let mut all = iterations.to_vec();
    all.push(UsageIteration {
        kind: UsageKind::Message,
        input_tokens: u64::try_from(usage.input_tokens).ok(),
        output_tokens: u64::try_from(usage.output_tokens).ok(),
    });
    usage.iterations = Some(all);
    usage
}

fn compaction_object(block: &CompactionBlock) -> Object {
    let mut fields = Object::new();
    fields.insert("type".to_owned(), Json::Text("compaction".to_owned()));
    if let Some(content) = &block.content {
        fields.insert("content".to_owned(), Json::Text(content.clone()));
    }
    if let Some(signature) = &block.signature {
        fields.insert("signature".to_owned(), Json::Text(signature.clone()));
    }
    fields
}
