use rust_decimal::Decimal;

use super::wire::{CompletionTokensDetails, DetailsField, PromptTokensDetails, UsageRecord};
use crate::stream_chunk_shaping::records::JsonValue;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecordedUsage {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) cache_creation_input_tokens: Option<i64>,
    pub(crate) cache_read_input_tokens: Option<i64>,
    pub(crate) completion_tokens_details: Option<CompletionTokensDetails>,
    pub(crate) prompt_tokens_details: Option<PromptTokensDetails>,
    pub(crate) server_tool_use: Option<JsonValue>,
    pub(crate) cost: Option<Decimal>,
    pub(crate) inference_geo: Option<JsonValue>,
    pub(crate) speed: Option<JsonValue>,
}

impl RecordedUsage {
    pub(crate) fn from_record(record: UsageRecord) -> Self {
        let mut completion_tokens_details = record
            .completion_tokens_details
            .and_then(DetailsField::present);
        fill_reasoning_tokens(
            &mut completion_tokens_details,
            record.reasoning_tokens,
            record.completion_tokens,
        );
        let mut prompt_tokens_details = record
            .prompt_tokens_details
            .and_then(DetailsField::present)
            .map(PromptTokensDetails::mirrored);
        if let Some(hits) = record.prompt_cache_hit_tokens {
            prompt_tokens_details.get_or_insert_default().cached_tokens = Some(hits);
        }
        if let Some(read) = record.cache_read_input_tokens {
            prompt_tokens_details.get_or_insert_default().cached_tokens = Some(read);
        }
        if let Some(creation) = record.cache_creation_input_tokens {
            let details = prompt_tokens_details.get_or_insert_default();
            details.cache_write_tokens = Some(creation);
            details.cache_creation_tokens = Some(creation);
        }
        Self {
            prompt_tokens: record.prompt_tokens.unwrap_or(0),
            completion_tokens: record.completion_tokens.unwrap_or(0),
            cache_creation_input_tokens: record.cache_creation_input_tokens,
            cache_read_input_tokens: record.cache_read_input_tokens,
            completion_tokens_details,
            prompt_tokens_details,
            server_tool_use: record.server_tool_use,
            cost: record.cost,
            inference_geo: record.inference_geo,
            speed: record.speed,
        }
    }
}

fn fill_reasoning_tokens(
    details: &mut Option<CompletionTokensDetails>,
    reasoning_tokens: Option<i64>,
    completion_tokens: Option<i64>,
) {
    let Some(reasoning) = reasoning_tokens.filter(|count| *count != 0) else {
        return;
    };
    let counted = details.get_or_insert_default();
    if counted.reasoning_tokens.is_none() {
        counted.reasoning_tokens = Some(reasoning);
    }
    let (Some(completion), None) = (completion_tokens, counted.text_tokens) else {
        return;
    };
    let mut text = completion.saturating_sub(reasoning);
    if let Some(image) = counted.image_tokens.filter(|count| *count != 0) {
        text = text.saturating_sub(image);
    }
    if let Some(audio) = counted.audio_tokens.filter(|count| *count != 0) {
        text = text.saturating_sub(audio);
    }
    counted.text_tokens = Some(text.max(0));
}
