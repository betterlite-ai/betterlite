use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails, Usage};

use crate::gemini_metadata::extract::extract_candidate_metadata;
use crate::gemini_usage::error::UsageError;
use crate::gemini_usage::metadata::{Modality, TokenCountDetail, UsageMetadata};
use crate::grounding_requests::calculate_grounding_requests;

#[derive(Default)]
struct ModalityCounts {
    text: Option<i64>,
    audio: Option<i64>,
    image: Option<i64>,
    video: Option<i64>,
}

pub fn calculate_usage(
    usage_metadata: Option<&UsageMetadata>,
    candidates: &[Json],
) -> Result<Usage, UsageError> {
    let Some(metadata) = usage_metadata else {
        return Err(UsageError::MissingUsageMetadata);
    };
    let completion_tokens = completion_tokens(metadata)?;
    Ok(Usage {
        prompt_tokens: prompt_tokens(metadata, candidates)?,
        completion_tokens,
        total_tokens: metadata.total_token_count,
        prompt_tokens_details: Some(prompt_details(metadata)?),
        completion_tokens_details: completion_details(metadata, completion_tokens)?,
        numeric_extras: BTreeMap::new(),
    })
}

pub fn apply_grounding_counters(usage: &mut Usage, grounding_metadata: &[Json]) {
    let counters = calculate_grounding_requests(grounding_metadata);
    let details = usage.prompt_tokens_details.get_or_insert_default();
    if let Some(requests) = counters.web_search_requests {
        details.web_search_requests = Some(requests);
    }
    if let Some(requests) = counters.google_maps_grounding_requests {
        details.google_maps_grounding_requests = Some(requests);
    }
}

fn prompt_tokens(metadata: &UsageMetadata, candidates: &[Json]) -> Result<i64, UsageError> {
    let searching = candidates.iter().any(|candidate| {
        let grounding = extract_candidate_metadata(candidate);
        let requests = calculate_grounding_requests(&grounding.grounding_metadata);
        requests.web_search_requests.is_some() || requests.google_maps_grounding_requests.is_some()
    });
    // Search and Maps grounding bill per request, so their retrieved tool tokens leave the prompt count
    let billable_tool_tokens = metadata
        .tool_use_prompt_token_count
        .filter(|_| !searching)
        .unwrap_or(0);
    metadata
        .prompt_token_count
        .checked_add(billable_tool_tokens)
        .ok_or(UsageError::Overflow {
            field: "prompt_tokens",
        })
}

fn prompt_details(metadata: &UsageMetadata) -> Result<PromptTokensDetails, UsageError> {
    let mut counts = ModalityCounts::default();
    fold_modalities(
        metadata
            .prompt_tokens_details
            .as_deref()
            .unwrap_or_default(),
        &mut counts,
        true,
    )?;
    let mut cached = ModalityCounts::default();
    fold_modalities(
        metadata.cache_tokens_details.as_deref().unwrap_or_default(),
        &mut cached,
        true,
    )?;
    subtract_cached(&mut counts, &cached, metadata)?;
    Ok(PromptTokensDetails {
        text_tokens: counts.text,
        audio_tokens: counts.audio,
        image_tokens: counts.image,
        video_tokens: counts.video,
        web_search_requests: None,
        google_maps_grounding_requests: None,
        // LiteLLM reads the count with `or None`, so a reported zero leaves the detail unset
        tool_use_tokens: metadata
            .tool_use_prompt_token_count
            .filter(|count| *count != 0),
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cached_tokens: metadata.cached_content_token_count,
        cached_tokens_details: None,
    })
}

fn completion_tokens(metadata: &UsageMetadata) -> Result<i64, UsageError> {
    let reported = metadata
        .response_token_count
        .filter(|count| *count != 0)
        .unwrap_or(metadata.candidates_token_count);
    let Some(reasoning) = metadata.thoughts_token_count.filter(|count| *count != 0) else {
        return Ok(reported);
    };
    if candidate_count_is_inclusive(metadata) {
        return Ok(reported);
    }
    reasoning.checked_add(reported).ok_or(UsageError::Overflow {
        field: "completion_tokens",
    })
}

fn completion_details(
    metadata: &UsageMetadata,
    completion_tokens: i64,
) -> Result<Option<CompletionTokensDetails>, UsageError> {
    let mut counts = ModalityCounts::default();
    let mut present = metadata.response_tokens_details.is_some()
        || metadata.candidates_tokens_details.is_some()
        || metadata.thoughts_token_count.is_some();
    if let Some(entries) = metadata.response_tokens_details.as_deref() {
        fold_modalities(entries, &mut counts, false)?;
    }
    if let Some(entries) = metadata.candidates_tokens_details.as_deref() {
        fold_modalities(entries, &mut counts, true)?;
    }
    let candidates_token_count = metadata.candidates_token_count;
    if candidates_token_count > 0 {
        present = true;
        if counts.text.is_none() {
            let others = [counts.image, counts.audio, counts.video];
            counts.text = Some(reduced(candidates_token_count, &others, "text_tokens")?);
        }
    }
    let mut details = present.then_some(CompletionTokensDetails {
        text_tokens: counts.text,
        audio_tokens: counts.audio,
        image_tokens: counts.image,
        video_tokens: counts.video,
        reasoning_tokens: metadata.thoughts_token_count,
        accepted_prediction_tokens: None,
        rejected_prediction_tokens: None,
    });
    // the Usage constructor subtracts reasoning, image and audio from the completion total and clamps at zero
    if let Some(reasoning) = metadata.thoughts_token_count.filter(|count| *count != 0)
        && let Some(completion) = details.as_mut()
        && completion.text_tokens.is_none()
    {
        let others = [
            Some(reasoning),
            completion.image_tokens,
            completion.audio_tokens,
        ];
        completion.text_tokens = Some(reduced(completion_tokens, &others, "text_tokens")?.max(0));
    }
    Ok(details)
}

fn candidate_count_is_inclusive(metadata: &UsageMetadata) -> bool {
    let sum = [metadata.prompt_token_count, metadata.candidates_token_count]
        .into_iter()
        .chain(metadata.tool_use_prompt_token_count)
        .try_fold(0_i128, |total, count| total.checked_add(i128::from(count)));
    sum == Some(i128::from(metadata.total_token_count))
}

fn fold_modalities(
    entries: &[TokenCountDetail],
    counts: &mut ModalityCounts,
    // responseTokensDetails has no IMAGE or VIDEO branch, unlike the candidates list
    include_media: bool,
) -> Result<(), UsageError> {
    for entry in entries {
        let count = entry.token_count().unwrap_or(0);
        let target = match Modality::from_wire(&entry.modality) {
            Modality::Text | Modality::Document => &mut counts.text,
            Modality::Audio => &mut counts.audio,
            Modality::Image if include_media => &mut counts.image,
            Modality::Video if include_media => &mut counts.video,
            Modality::Image | Modality::Video | Modality::Other => continue,
        };
        accumulate(target, count, "usage detail")?;
    }
    Ok(())
}

fn accumulate(total: &mut Option<i64>, count: i64, field: &'static str) -> Result<(), UsageError> {
    let current = total.unwrap_or(0);
    *total = Some(
        current
            .checked_add(count)
            .ok_or(UsageError::Overflow { field })?,
    );
    Ok(())
}

fn reduced(base: i64, parts: &[Option<i64>], field: &'static str) -> Result<i64, UsageError> {
    let mut value = base;
    for part in parts.iter().flatten() {
        value = value
            .checked_sub(*part)
            .ok_or(UsageError::Overflow { field })?;
    }
    Ok(value)
}

fn subtract_cached(
    counts: &mut ModalityCounts,
    cached: &ModalityCounts,
    metadata: &UsageMetadata,
) -> Result<(), UsageError> {
    let implicit_cached = metadata
        .cached_content_token_count
        .filter(|_| metadata.cache_tokens_details.is_none());
    if let (Some(text), Some(cached_text)) = (counts.text, cached.text.or(implicit_cached)) {
        counts.text = Some(text.checked_sub(cached_text).ok_or(UsageError::Overflow {
            field: "text_tokens",
        })?);
    }
    subtract_pair(&mut counts.audio, cached.audio, "audio_tokens")?;
    subtract_pair(&mut counts.image, cached.image, "image_tokens")?;
    subtract_pair(&mut counts.video, cached.video, "video_tokens")
}

fn subtract_pair(
    current: &mut Option<i64>,
    cached: Option<i64>,
    field: &'static str,
) -> Result<(), UsageError> {
    if let (Some(value), Some(cached_value)) = (*current, cached) {
        *current = Some(
            value
                .checked_sub(cached_value)
                .ok_or(UsageError::Overflow { field })?,
        );
    }
    Ok(())
}
