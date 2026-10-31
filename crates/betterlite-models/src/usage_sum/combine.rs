use rust_decimal::Decimal;

use super::{CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, UsageSumError};

pub(super) fn combine_cached_tokens_details(
    current: Option<&CachedTokensDetails>,
    new: &CachedTokensDetails,
) -> Result<CachedTokensDetails, UsageSumError> {
    let text_tokens = sum_absent_preserving(
        current.and_then(|value| value.text_tokens),
        new.text_tokens,
        "cached_tokens_details.text_tokens",
    )?;
    Ok(CachedTokensDetails {
        text_tokens,
        audio_tokens: sum_absent_preserving(
            current.and_then(|value| value.audio_tokens),
            new.audio_tokens,
            "cached_tokens_details.audio_tokens",
        )?,
        image_tokens: sum_absent_preserving(
            current.and_then(|value| value.image_tokens),
            new.image_tokens,
            "cached_tokens_details.image_tokens",
        )?,
    })
}

pub(super) fn combine_prompt_tokens_details(
    current: Option<&PromptTokensDetails>,
    new: &PromptTokensDetails,
) -> Result<PromptTokensDetails, UsageSumError> {
    let base = current.cloned().unwrap_or_default();
    let (cache_write_tokens, cache_creation_tokens) = combine_cache_aliases(&base, new)?;
    let cached_tokens_details = combine_cached_modalities(&base, new)?;
    let text_tokens = sum_always(base.text_tokens, new.text_tokens, "text_tokens")?;
    let audio_tokens = sum_always(base.audio_tokens, new.audio_tokens, "audio_tokens")?;
    let image_tokens = sum_always(base.image_tokens, new.image_tokens, "image_tokens")?;
    let video_tokens = sum_always(base.video_tokens, new.video_tokens, "video_tokens")?;
    let counts = combine_absent_counts(&base, new)?;
    Ok(PromptTokensDetails {
        text_tokens,
        audio_tokens,
        image_tokens,
        video_tokens,
        web_search_requests: counts.web_search_requests,
        google_maps_grounding_requests: counts.google_maps_grounding_requests,
        tool_use_tokens: counts.tool_use_tokens,
        character_count: counts.character_count,
        image_count: counts.image_count,
        video_length_seconds: counts.video_length_seconds,
        audio_length_seconds: counts.audio_length_seconds,
        query_count: counts.query_count,
        cache_write_tokens,
        cache_creation_tokens,
        cached_tokens_details,
    })
}

struct AbsentPreservingCounts {
    web_search_requests: Option<i64>,
    google_maps_grounding_requests: Option<i64>,
    tool_use_tokens: Option<i64>,
    character_count: Option<i64>,
    image_count: Option<i64>,
    query_count: Option<i64>,
    video_length_seconds: Option<Decimal>,
    audio_length_seconds: Option<Decimal>,
}

fn combine_absent_counts(
    base: &PromptTokensDetails,
    new: &PromptTokensDetails,
) -> Result<AbsentPreservingCounts, UsageSumError> {
    let web_search_requests = sum_absent_preserving(
        base.web_search_requests,
        new.web_search_requests,
        "web_search_requests",
    )?;
    Ok(AbsentPreservingCounts {
        web_search_requests,
        google_maps_grounding_requests: sum_absent_preserving(
            base.google_maps_grounding_requests,
            new.google_maps_grounding_requests,
            "google_maps_grounding_requests",
        )?,
        tool_use_tokens: sum_absent_preserving(
            base.tool_use_tokens,
            new.tool_use_tokens,
            "tool_use_tokens",
        )?,
        character_count: sum_absent_preserving(
            base.character_count,
            new.character_count,
            "character_count",
        )?,
        image_count: sum_absent_preserving(base.image_count, new.image_count, "image_count")?,
        query_count: sum_absent_preserving(base.query_count, new.query_count, "query_count")?,
        video_length_seconds: sum_absent_preserving(
            base.video_length_seconds,
            new.video_length_seconds,
            "video_length_seconds",
        )?,
        audio_length_seconds: sum_absent_preserving(
            base.audio_length_seconds,
            new.audio_length_seconds,
            "audio_length_seconds",
        )?,
    })
}

fn combine_cached_modalities(
    base: &PromptTokensDetails,
    new: &PromptTokensDetails,
) -> Result<Option<CachedTokensDetails>, UsageSumError> {
    let current = base.cached_tokens_details.as_ref();
    new.cached_tokens_details.as_ref().map_or_else(
        || Ok(base.cached_tokens_details.clone()),
        |value| combine_cached_tokens_details(current, value).map(Some),
    )
}

fn combine_cache_aliases(
    base: &PromptTokensDetails,
    new: &PromptTokensDetails,
) -> Result<(Option<i64>, Option<i64>), UsageSumError> {
    let new_cache_write = new.cache_write_tokens.or(new.cache_creation_tokens);
    let Some(value) = new_cache_write else {
        return Ok((base.cache_write_tokens, base.cache_creation_tokens));
    };
    let summed = sum_always(base.cache_write_tokens, Some(value), "cache_write_tokens")?;
    Ok((summed, summed))
}

pub(super) fn sum_completion_details(
    combined: &mut CompletionTokensDetails,
    new: &CompletionTokensDetails,
) -> Result<(), UsageSumError> {
    combined.text_tokens = sum_always(
        combined.text_tokens,
        new.text_tokens,
        "completion_tokens_details.text_tokens",
    )?;
    combined.audio_tokens = sum_always(
        combined.audio_tokens,
        new.audio_tokens,
        "completion_tokens_details.audio_tokens",
    )?;
    combined.image_tokens = sum_always(
        combined.image_tokens,
        new.image_tokens,
        "completion_tokens_details.image_tokens",
    )?;
    combined.video_tokens = sum_always(
        combined.video_tokens,
        new.video_tokens,
        "completion_tokens_details.video_tokens",
    )?;
    combined.reasoning_tokens = sum_always(
        combined.reasoning_tokens,
        new.reasoning_tokens,
        "completion_tokens_details.reasoning_tokens",
    )?;
    combined.accepted_prediction_tokens = sum_always(
        combined.accepted_prediction_tokens,
        new.accepted_prediction_tokens,
        "completion_tokens_details.accepted_prediction_tokens",
    )?;
    combined.rejected_prediction_tokens = sum_always(
        combined.rejected_prediction_tokens,
        new.rejected_prediction_tokens,
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

macro_rules! checked_sum_fn {
    (fn $name:ident, absent $absent:expr) => {
        fn $name<T>(
            current: Option<T>,
            new: Option<T>,
            field: &str,
        ) -> Result<Option<T>, UsageSumError>
        where
            T: CheckedCounter,
        {
            new.or($absent).map_or(Ok(current), |value| {
                current
                    .unwrap_or(T::ZERO)
                    .checked_sum(value)
                    .map(Some)
                    .ok_or_else(|| T::overflow(field.to_owned()))
            })
        }
    };
}

checked_sum_fn!(fn sum_absent_preserving, absent None);
checked_sum_fn!(fn sum_always, absent Some(T::ZERO));
