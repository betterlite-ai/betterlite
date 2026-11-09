use std::borrow::Cow;

use rust_decimal::Decimal;

use super::UsageAssemblyError;
use super::details::{attach_cache_breakdown, capture_cache_breakdown};
use super::wire::{
    CacheCreationTokenDetails, Chunk, ChunkForm, CompletionTokensDetails, PromptTokensDetails,
    ServerToolUse, UsageRecord, UsageSlot,
};
use crate::stream_chunk_shaping::records::JsonValue;

// The Usage object LiteLLM builds from one usage record before the accumulator reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecordedUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    completion_tokens_details: Option<CompletionTokensDetails>,
    prompt_tokens_details: Option<PromptTokensDetails>,
    server_tool_use: Option<JsonValue>,
    cost: Option<Decimal>,
    inference_geo: Option<JsonValue>,
    speed: Option<JsonValue>,
}

impl RecordedUsage {
    // Usage(**record) applies the constructor's cache and reasoning mappings before any accumulator sees it.
    fn from_record(record: UsageRecord) -> Self {
        let mut completion_tokens_details = record.completion_tokens_details;
        fill_reasoning_tokens(
            &mut completion_tokens_details,
            record.reasoning_tokens,
            record.completion_tokens,
        );
        let mut prompt_tokens_details = record
            .prompt_tokens_details
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

// The owned usage-summary extraction: absent or null counts read as a reported 0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UsageSummary {
    prompt_tokens: i64,
    completion_tokens: i64,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    completion_tokens_details: Option<CompletionTokensDetails>,
    prompt_tokens_details: Option<PromptTokensDetails>,
    cost: Option<Decimal>,
}

#[must_use]
pub(crate) fn usage_chunk_calculation_helper(record: &RecordedUsage) -> UsageSummary {
    UsageSummary {
        prompt_tokens: record.prompt_tokens,
        completion_tokens: record.completion_tokens,
        cache_creation_input_tokens: record.cache_creation_input_tokens,
        cache_read_input_tokens: record.cache_read_input_tokens,
        completion_tokens_details: record.completion_tokens_details.clone(),
        prompt_tokens_details: record.prompt_tokens_details.clone(),
        cost: record.cost,
    }
}

#[must_use]
pub(crate) fn reports_prompt_side_usage(summary: &UsageSummary) -> bool {
    summary.prompt_tokens > 0
        || summary
            .cache_creation_input_tokens
            .is_some_and(|count| count > 0)
        || summary
            .cache_read_input_tokens
            .is_some_and(|count| count > 0)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PricingField {
    InferenceGeo,
    Speed,
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

#[must_use]
pub(crate) fn last_provider_pricing_field(chunks: &[Chunk], field: PricingField) -> Option<String> {
    let mut last = None;
    for chunk in chunks {
        let Some(record) = extract_usage_chunk(chunk) else {
            continue;
        };
        let value = match field {
            PricingField::InferenceGeo => record.inference_geo,
            PricingField::Speed => record.speed,
        };
        let Some(text) = value.as_ref().and_then(JsonValue::as_str) else {
            continue;
        };
        last = Some(text.to_owned());
    }
    last
}

#[must_use]
pub(crate) fn saw_finish_reason(chunks: &[Chunk]) -> bool {
    chunks.iter().any(|chunk| {
        chunk
            .choices
            .iter()
            .any(|choice| choice.finish_reason.is_some())
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
            other => {
                return Err(UsageAssemblyError::ServerToolUse {
                    field: other.to_owned(),
                });
            }
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

#[derive(Debug, Clone, Copy)]
enum CacheCounter {
    Creation,
    Read,
}

#[derive(Debug, Default)]
struct Accumulator {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    completion_usage_updates: u32,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    server_tool_use: Option<ServerToolUse>,
    web_search_requests: Option<i64>,
    google_maps_grounding_requests: Option<i64>,
    completion_tokens_details: Option<CompletionTokensDetails>,
    prompt_tokens_details: Option<PromptTokensDetails>,
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
    cost: Option<Decimal>,
}

impl Accumulator {
    fn absorb(&mut self, record: &RecordedUsage) -> Result<(), UsageAssemblyError> {
        let summary = usage_chunk_calculation_helper(record);
        self.absorb_counts(&summary);
        match record.server_tool_use.as_ref() {
            None | Some(JsonValue::Null) => {}
            Some(value) => self.server_tool_use = Some(server_tool_use(value)?),
        }
        self.absorb_details(summary);
        Ok(())
    }

    fn absorb_counts(&mut self, summary: &UsageSummary) {
        if summary.prompt_tokens > 0 || self.prompt_tokens.is_none() {
            self.prompt_tokens = Some(summary.prompt_tokens);
        }
        if summary.completion_tokens > 0 || self.completion_tokens.is_none() {
            self.completion_tokens = Some(summary.completion_tokens);
        }
        if summary.completion_tokens > 0 {
            self.completion_usage_updates = self.completion_usage_updates.saturating_add(1);
        }
        let reports_prompt_side = reports_prompt_side_usage(summary);
        self.absorb_cache_count(
            summary.cache_creation_input_tokens,
            reports_prompt_side,
            CacheCounter::Creation,
        );
        self.absorb_cache_count(
            summary.cache_read_input_tokens,
            reports_prompt_side,
            CacheCounter::Read,
        );
    }

    const fn absorb_cache_count(
        &mut self,
        count: Option<i64>,
        reports_prompt_side: bool,
        counter: CacheCounter,
    ) {
        let Some(amount) = count else {
            return;
        };
        let slot = match counter {
            CacheCounter::Creation => &mut self.cache_creation_input_tokens,
            CacheCounter::Read => &mut self.cache_read_input_tokens,
        };
        if reports_prompt_side || slot.is_none() {
            *slot = Some(amount);
        }
    }

    fn absorb_details(&mut self, summary: UsageSummary) {
        if let Some(details) = summary.completion_tokens_details {
            self.completion_tokens_details = Some(details);
        }
        if let Some(details) = summary.prompt_tokens_details {
            if let Some(requests) = details.web_search_requests {
                self.web_search_requests = Some(requests);
            }
            if let Some(requests) = details.google_maps_grounding_requests {
                self.google_maps_grounding_requests = Some(requests);
            }
            self.prompt_tokens_details = Some(details);
        }
        let current = self.cache_creation_token_details.take();
        self.cache_creation_token_details =
            capture_cache_breakdown(self.prompt_tokens_details.as_ref(), current);
        if let Some(cost) = summary.cost {
            self.cost = Some(cost);
        }
    }

    fn finish(self, chunks: &[Chunk]) -> UsagePerChunk {
        let recovered = reset_cursor_completion(
            chunks,
            self.completion_tokens,
            self.completion_usage_updates,
        );
        let cursor_was_reset = recovered != self.completion_tokens;
        let prompt_tokens_details = attach_cache_breakdown(
            self.prompt_tokens_details.as_ref(),
            self.cache_creation_token_details.as_ref(),
        )
        .map(Cow::into_owned);
        UsagePerChunk {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: recovered,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens,
            server_tool_use: self.server_tool_use,
            web_search_requests: self.web_search_requests,
            google_maps_grounding_requests: self.google_maps_grounding_requests,
            completion_tokens_details: if cursor_was_reset {
                None
            } else {
                self.completion_tokens_details
            },
            prompt_tokens_details,
            cost: self.cost,
            inference_geo: last_provider_pricing_field(chunks, PricingField::InferenceGeo),
            speed: last_provider_pricing_field(chunks, PricingField::Speed),
        }
    }
}

pub(crate) fn calculate_usage_per_chunk(
    chunks: &[Chunk],
) -> Result<UsagePerChunk, UsageAssemblyError> {
    let mut accumulator = Accumulator::default();
    for chunk in chunks {
        if let Some(record) = extract_usage_chunk(chunk) {
            accumulator.absorb(&record)?;
        }
    }
    Ok(accumulator.finish(chunks))
}
