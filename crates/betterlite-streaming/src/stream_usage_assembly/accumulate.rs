use std::borrow::Cow;

use rust_decimal::Decimal;

use super::UsageAssemblyError;
use super::details::{attach_cache_breakdown, capture_cache_breakdown};
use super::record::RecordedUsage;
use super::wire::{
    CacheCreationTokenDetails, Chunk, ChunkForm, CompletionTokensDetails, PromptTokensDetails,
    ServerToolUse, UsageSlot,
};
use crate::stream_chunk_shaping::records::JsonValue;

#[derive(Debug, Default)]
pub(crate) struct UsagePerChunk {
    pub(crate) prompt_tokens: Option<i64>,
    pub(crate) completion_tokens: Option<i64>,
    pub(crate) cache_creation_input_tokens: Option<i64>,
    pub(crate) cache_read_input_tokens: Option<i64>,
    pub(crate) server_tool_use: Option<ServerToolUse>,
    pub(crate) web_search_requests: Option<i64>,
    pub(crate) google_maps_grounding_requests: Option<i64>,
    pub(crate) completion_tokens_details: Option<CompletionTokensDetails>,
    pub(crate) prompt_tokens_details: Option<PromptTokensDetails>,
    pub(crate) cost: Option<Decimal>,
    pub(crate) inference_geo: Option<String>,
    pub(crate) speed: Option<String>,
    completion_usage_updates: u32,
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
}

#[must_use]
pub(crate) fn reports_prompt_side_usage(record: &RecordedUsage) -> bool {
    record.prompt_tokens > 0
        || record
            .cache_creation_input_tokens
            .is_some_and(|count| count > 0)
        || record
            .cache_read_input_tokens
            .is_some_and(|count| count > 0)
}

// A dict chunk only reads its usage key, while an SDK object with no usage attribute falls back to hidden params.
#[must_use]
pub(crate) fn extract_usage_chunk(chunk: &Chunk) -> Option<RecordedUsage> {
    match (&chunk.form, &chunk.usage) {
        (_, UsageSlot::Key(Some(record))) => {
            Some(RecordedUsage::from_record(record.as_ref().clone()))
        }
        (ChunkForm::Dict, _) | (_, UsageSlot::Key(None)) => None,
        (ChunkForm::ModelResponse | ChunkForm::ModelResponseStream, UsageSlot::Absent) => chunk
            .hidden_params
            .usage
            .clone()
            .map(RecordedUsage::from_record),
    }
}

// Both provider pricing fields keep their last string value, whatever their magnitude.
#[must_use]
pub(crate) fn last_pricing_fields(chunks: &[Chunk]) -> (Option<String>, Option<String>) {
    let mut inference_geo = None;
    let mut speed = None;
    for chunk in chunks {
        let Some(record) = extract_usage_chunk(chunk) else {
            continue;
        };
        if let Some(text) = record.inference_geo.as_ref().and_then(JsonValue::as_str) {
            inference_geo = Some(text.to_owned());
        }
        if let Some(text) = record.speed.as_ref().and_then(JsonValue::as_str) {
            speed = Some(text.to_owned());
        }
    }
    (inference_geo, speed)
}

// Choices defaults a missing or null finish_reason to "stop", so every choice on a ModelResponse is evidence.
#[must_use]
pub(crate) fn saw_finish_reason(chunks: &[Chunk]) -> bool {
    chunks.iter().any(|chunk| match chunk.form {
        ChunkForm::ModelResponse => !chunk.choices.is_empty(),
        ChunkForm::Dict | ChunkForm::ModelResponseStream => chunk
            .choices
            .iter()
            .any(|choice| choice.finish_reason.is_some()),
    })
}

#[must_use]
pub(crate) fn reset_cursor_completion(
    chunks: &[Chunk],
    completion_tokens: Option<i64>,
    completion_usage_updates: u32,
) -> Option<i64> {
    if completion_usage_updates >= 2 || saw_finish_reason(chunks) {
        return completion_tokens;
    }
    let provider = chunks
        .first()
        .and_then(|chunk| chunk.hidden_params.custom_llm_provider.as_deref());
    if provider == Some("anthropic") {
        return None;
    }
    completion_tokens
}

pub(crate) fn server_tool_use(raw: &JsonValue) -> Result<ServerToolUse, UsageAssemblyError> {
    let JsonValue::Object(fields) = raw else {
        return Err(UsageAssemblyError::ServerToolUse {
            field: "server_tool_use".to_owned(),
        });
    };
    let mut parsed = ServerToolUse::default();
    for (name, value) in fields {
        let slot = match name.as_str() {
            "web_search_requests" => &mut parsed.web_search_requests,
            "tool_search_requests" => &mut parsed.tool_search_requests,
            "browser_open_requests" => &mut parsed.browser_open_requests,
            _other => continue,
        };
        if !matches!(value, JsonValue::Null) {
            *slot = Some(
                value
                    .as_i64()
                    .ok_or_else(|| UsageAssemblyError::ServerToolUse {
                        field: name.clone(),
                    })?,
            );
        }
    }
    Ok(parsed)
}

impl UsagePerChunk {
    fn absorb(&mut self, record: &RecordedUsage) -> Result<(), UsageAssemblyError> {
        self.absorb_counts(record);
        match record.server_tool_use.as_ref() {
            None | Some(JsonValue::Null) => {}
            Some(value) => self.server_tool_use = Some(server_tool_use(value)?),
        }
        self.absorb_details(record);
        Ok(())
    }

    fn absorb_counts(&mut self, record: &RecordedUsage) {
        if record.prompt_tokens > 0 || self.prompt_tokens.is_none() {
            self.prompt_tokens = Some(record.prompt_tokens);
        }
        if record.completion_tokens > 0 || self.completion_tokens.is_none() {
            self.completion_tokens = Some(record.completion_tokens);
        }
        if record.completion_tokens > 0 {
            self.completion_usage_updates = self.completion_usage_updates.saturating_add(1);
        }
        let reports_prompt_side = reports_prompt_side_usage(record);
        if let Some(count) = record.cache_creation_input_tokens
            && (reports_prompt_side || self.cache_creation_input_tokens.is_none())
        {
            self.cache_creation_input_tokens = Some(count);
        }
        if let Some(count) = record.cache_read_input_tokens
            && (reports_prompt_side || self.cache_read_input_tokens.is_none())
        {
            self.cache_read_input_tokens = Some(count);
        }
    }

    fn absorb_details(&mut self, record: &RecordedUsage) {
        if let Some(details) = record.completion_tokens_details.clone() {
            self.completion_tokens_details = Some(details);
        }
        if let Some(details) = record.prompt_tokens_details.clone() {
            self.web_search_requests = details.web_search_requests.or(self.web_search_requests);
            self.google_maps_grounding_requests = details
                .google_maps_grounding_requests
                .or(self.google_maps_grounding_requests);
            self.prompt_tokens_details = Some(details);
        }
        let current = self.cache_creation_token_details.take();
        self.cache_creation_token_details =
            capture_cache_breakdown(self.prompt_tokens_details.as_ref(), current);
        if let Some(cost) = record.cost {
            self.cost = Some(cost);
        }
    }

    fn finish(mut self, chunks: &[Chunk]) -> Self {
        let recovered = reset_cursor_completion(
            chunks,
            self.completion_tokens,
            self.completion_usage_updates,
        );
        if recovered != self.completion_tokens {
            self.completion_tokens_details = None;
        }
        self.completion_tokens = recovered;
        let attached = attach_cache_breakdown(
            self.prompt_tokens_details.as_ref(),
            self.cache_creation_token_details.as_ref(),
        )
        .map(Cow::into_owned);
        self.prompt_tokens_details = attached;
        let (inference_geo, speed) = last_pricing_fields(chunks);
        self.inference_geo = inference_geo;
        self.speed = speed;
        self
    }
}

pub(crate) fn calculate_usage_per_chunk(
    chunks: &[Chunk],
) -> Result<UsagePerChunk, UsageAssemblyError> {
    let mut usage = UsagePerChunk::default();
    for chunk in chunks {
        if let Some(record) = extract_usage_chunk(chunk) {
            usage.absorb(&record)?;
        }
    }
    Ok(usage.finish(chunks))
}
