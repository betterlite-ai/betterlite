use rust_decimal::Decimal;

use super::{CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, UsageSumError};

struct CoreCounters {
    text: Option<i64>,
    audio: Option<i64>,
    image: Option<i64>,
    video: Option<i64>,
    cached: Option<i64>,
}

struct ExtraCounters {
    web_search_requests: Option<i64>,
    google_maps_grounding_requests: Option<i64>,
    tool_use_tokens: Option<i64>,
    character_count: Option<i64>,
    image_count: Option<i64>,
    query_count: Option<i64>,
    video_length_seconds: Option<Decimal>,
    audio_length_seconds: Option<Decimal>,
}

pub(super) fn combine_prompt_tokens_details(
    current: Option<&PromptTokensDetails>,
    new: &PromptTokensDetails,
) -> Result<PromptTokensDetails, UsageSumError> {
    let base = current.cloned().unwrap_or_default();
    let (cache_write_tokens, cache_creation_tokens) =
        combine_cache_aliases(&base, new.cache_write_tokens.or(new.cache_creation_tokens))?;
    let cached_tokens_details = combine_cached_tokens_details(
        base.cached_tokens_details.as_ref(),
        new.cached_tokens_details.as_ref(),
    )?;
    let core = combine_core_counters(&base, new)?;
    let extras = combine_extra_counters(&base, new)?;
    Ok(PromptTokensDetails {
        text_tokens: core.text,
        audio_tokens: core.audio,
        image_tokens: core.image,
        video_tokens: core.video,
        cached_tokens: core.cached,
        web_search_requests: extras.web_search_requests,
        google_maps_grounding_requests: extras.google_maps_grounding_requests,
        tool_use_tokens: extras.tool_use_tokens,
        character_count: extras.character_count,
        image_count: extras.image_count,
        query_count: extras.query_count,
        video_length_seconds: extras.video_length_seconds,
        audio_length_seconds: extras.audio_length_seconds,
        cache_write_tokens,
        cache_creation_tokens,
        cached_tokens_details,
    })
}

fn combine_core_counters(
    base: &PromptTokensDetails,
    new: &PromptTokensDetails,
) -> Result<CoreCounters, UsageSumError> {
    let text = base
        .text_tokens
        .sum_absent_preserving(new.text_tokens.or(Some(0)), "text_tokens")?;
    let audio = base
        .audio_tokens
        .sum_absent_preserving(new.audio_tokens.or(Some(0)), "audio_tokens")?;
    let image = base
        .image_tokens
        .sum_absent_preserving(new.image_tokens.or(Some(0)), "image_tokens")?;
    let video = base
        .video_tokens
        .sum_absent_preserving(new.video_tokens.or(Some(0)), "video_tokens")?;
    let cached = base
        .cached_tokens
        .sum_absent_preserving(new.cached_tokens.or(Some(0)), "cached_tokens")?;
    Ok(CoreCounters {
        text,
        audio,
        image,
        video,
        cached,
    })
}

fn combine_extra_counters(
    base: &PromptTokensDetails,
    new: &PromptTokensDetails,
) -> Result<ExtraCounters, UsageSumError> {
    let web_search_requests = base
        .web_search_requests
        .sum_absent_preserving(new.web_search_requests, "web_search_requests")?;
    let google_maps_grounding_requests =
        base.google_maps_grounding_requests.sum_absent_preserving(
            new.google_maps_grounding_requests,
            "google_maps_grounding_requests",
        )?;
    let tool_use_tokens = base
        .tool_use_tokens
        .sum_absent_preserving(new.tool_use_tokens, "tool_use_tokens")?;
    let character_count = base
        .character_count
        .sum_absent_preserving(new.character_count, "character_count")?;
    let image_count = base
        .image_count
        .sum_absent_preserving(new.image_count, "image_count")?;
    let query_count = base
        .query_count
        .sum_absent_preserving(new.query_count, "query_count")?;
    let video_length_seconds = base
        .video_length_seconds
        .sum_absent_preserving(new.video_length_seconds, "video_length_seconds")?;
    let audio_length_seconds = base
        .audio_length_seconds
        .sum_absent_preserving(new.audio_length_seconds, "audio_length_seconds")?;
    Ok(ExtraCounters {
        web_search_requests,
        google_maps_grounding_requests,
        tool_use_tokens,
        character_count,
        image_count,
        query_count,
        video_length_seconds,
        audio_length_seconds,
    })
}

fn combine_cached_tokens_details(
    base: Option<&CachedTokensDetails>,
    new: Option<&CachedTokensDetails>,
) -> Result<Option<CachedTokensDetails>, UsageSumError> {
    let Some(value) = new else {
        return Ok(base.cloned());
    };
    let text_tokens = base
        .and_then(|cached| cached.text_tokens)
        .sum_absent_preserving(value.text_tokens, "cached_tokens_details.text_tokens")?;
    let audio_tokens = base
        .and_then(|cached| cached.audio_tokens)
        .sum_absent_preserving(value.audio_tokens, "cached_tokens_details.audio_tokens")?;
    let image_tokens = base
        .and_then(|cached| cached.image_tokens)
        .sum_absent_preserving(value.image_tokens, "cached_tokens_details.image_tokens")?;
    Ok(Some(CachedTokensDetails {
        text_tokens,
        audio_tokens,
        image_tokens,
    }))
}

fn combine_cache_aliases(
    base: &PromptTokensDetails,
    new_cache_write: Option<i64>,
) -> Result<(Option<i64>, Option<i64>), UsageSumError> {
    let Some(value) = new_cache_write else {
        return Ok((base.cache_write_tokens, base.cache_creation_tokens));
    };
    let summed = base
        .cache_write_tokens
        .sum_absent_preserving(Some(value), "cache_write_tokens")?;
    Ok((summed, summed))
}

pub(super) fn sum_completion_details(
    combined: &mut CompletionTokensDetails,
    new: &CompletionTokensDetails,
) -> Result<(), UsageSumError> {
    combined.text_tokens = combined.text_tokens.sum_absent_preserving(
        new.text_tokens.or(Some(0)),
        "completion_tokens_details.text_tokens",
    )?;
    combined.audio_tokens = combined.audio_tokens.sum_absent_preserving(
        new.audio_tokens.or(Some(0)),
        "completion_tokens_details.audio_tokens",
    )?;
    combined.image_tokens = combined.image_tokens.sum_absent_preserving(
        new.image_tokens.or(Some(0)),
        "completion_tokens_details.image_tokens",
    )?;
    combined.video_tokens = combined.video_tokens.sum_absent_preserving(
        new.video_tokens.or(Some(0)),
        "completion_tokens_details.video_tokens",
    )?;
    combined.reasoning_tokens = combined.reasoning_tokens.sum_absent_preserving(
        new.reasoning_tokens.or(Some(0)),
        "completion_tokens_details.reasoning_tokens",
    )?;
    combined.accepted_prediction_tokens =
        combined.accepted_prediction_tokens.sum_absent_preserving(
            new.accepted_prediction_tokens.or(Some(0)),
            "completion_tokens_details.accepted_prediction_tokens",
        )?;
    combined.rejected_prediction_tokens =
        combined.rejected_prediction_tokens.sum_absent_preserving(
            new.rejected_prediction_tokens.or(Some(0)),
            "completion_tokens_details.rejected_prediction_tokens",
        )?;
    Ok(())
}

trait CheckedCounter: Copy {
    const ZERO: Self;

    fn checked_sum(self, other: Self) -> Option<Self>;

    fn overflow(field: String) -> UsageSumError;
}

impl CheckedCounter for i64 {
    const ZERO: Self = 0;

    fn checked_sum(self, other: Self) -> Option<Self> {
        self.checked_add(other)
    }

    fn overflow(field: String) -> UsageSumError {
        UsageSumError::IntegerOverflow { field }
    }
}

const DECIMAL_ZERO: Decimal = Decimal::ZERO;

impl CheckedCounter for Decimal {
    const ZERO: Self = DECIMAL_ZERO;

    fn checked_sum(self, other: Self) -> Option<Self> {
        self.checked_add(other)
    }

    fn overflow(field: String) -> UsageSumError {
        UsageSumError::DecimalOverflow { field }
    }
}

trait AbsentPreservingSum: Sized {
    fn sum_absent_preserving(self, new: Self, field: &str) -> Result<Self, UsageSumError>;
}

impl<T: CheckedCounter> AbsentPreservingSum for Option<T> {
    fn sum_absent_preserving(self, new: Self, field: &str) -> Result<Self, UsageSumError> {
        new.map_or(Ok(self), |value| {
            self.unwrap_or(T::ZERO)
                .checked_sum(value)
                .map(Some)
                .ok_or_else(|| T::overflow(field.to_owned()))
        })
    }
}
