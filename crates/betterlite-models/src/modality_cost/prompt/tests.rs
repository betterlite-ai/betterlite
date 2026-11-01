#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{PromptTokensDetailsResult, get_billable_input_tokens, parse_prompt_tokens_details};
use crate::cost_usage::PromptDetails;
use crate::cost_usage::modality_projection::{
    CacheCreationTokenDetails, CachedTokenCounters, PromptModalityUsage,
};

fn empty_base() -> PromptDetails {
    PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
    }
}

fn empty_counters() -> CachedTokenCounters {
    CachedTokenCounters {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
    }
}

fn empty_usage() -> PromptModalityUsage {
    PromptModalityUsage {
        base: empty_base(),
        cached_tokens: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cache_creation_token_details: None,
        cached_tokens_details: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
    }
}

fn usage(
    base: PromptDetails,
    cached_tokens: i64,
    details: CachedTokenCounters,
) -> PromptModalityUsage {
    PromptModalityUsage {
        base,
        cached_tokens: Some(cached_tokens),
        cached_tokens_details: Some(details),
        ..empty_usage()
    }
}

#[test]
fn cache_allocates_audio_first_then_text_then_image() {
    let base = PromptDetails {
        text_tokens: Some(10),
        audio_tokens: Some(5),
        image_tokens: Some(4),
        ..empty_base()
    };
    let details = CachedTokenCounters {
        audio_tokens: Some(3),
        text_tokens: Some(4),
        image_tokens: Some(10),
    };
    let result = parse_prompt_tokens_details(&usage(base, 8, details)).expect("no overflow");
    assert_eq!(result.cache_hit_audio_tokens, 3);
    assert_eq!(result.audio_tokens, 2);
    assert_eq!(result.text_tokens, 6);
    // Only 1 token of cache hit remains for image (8 - 3 audio - 4 text), capped below the 10 requested.
    assert_eq!(result.image_tokens, 3);
}

#[test]
fn ordinary_modalities_clamp_at_zero_when_cache_exceeds_them() {
    let base = PromptDetails {
        text_tokens: Some(2),
        ..empty_base()
    };
    let details = CachedTokenCounters {
        text_tokens: Some(9),
        ..empty_counters()
    };
    let result = parse_prompt_tokens_details(&usage(base, 9, details)).expect("no overflow");
    assert_eq!(result.text_tokens, 0);
}

#[test]
fn negative_reported_modality_is_retained_through_subtraction() {
    let base = PromptDetails {
        text_tokens: Some(-5),
        ..empty_base()
    };
    let result = parse_prompt_tokens_details(&usage(base, 0, empty_counters())).expect("ok");
    assert_eq!(
        result.text_tokens, 0,
        "clamped at zero after the subtraction"
    );
}

#[rstest]
#[case::absent_defaults_to_zero(None, 0)]
#[case::zero_stays_zero(Some(0), 0)]
#[case::negative_collapses_to_zero(Some(-3), 0)]
#[case::positive_is_kept(Some(4), 4)]
fn video_tokens_require_a_strictly_positive_count(
    #[case] reported: Option<i64>,
    #[case] expected: i64,
) {
    let base = PromptDetails {
        video_tokens: reported,
        ..empty_base()
    };
    let result = parse_prompt_tokens_details(&usage(base, 0, empty_counters())).expect("ok");
    assert_eq!(result.video_tokens, expected);
}

#[rstest]
#[case::absent_defaults_to_zero(None, 0)]
#[case::zero_stays_zero(Some(0), 0)]
#[case::negative_collapses_to_zero(Some(-3), 0)]
#[case::positive_is_kept(Some(4), 4)]
fn query_count_requires_a_strictly_positive_count(
    #[case] reported: Option<i64>,
    #[case] expected: i64,
) {
    let present = PromptModalityUsage {
        query_count: reported,
        ..empty_usage()
    };
    let result = parse_prompt_tokens_details(&present).expect("ok");
    assert_eq!(result.query_count, expected);
}

#[test]
fn character_and_duration_fields_default_to_zero_when_absent() {
    let result = parse_prompt_tokens_details(&empty_usage()).expect("ok");
    assert_eq!(result.character_count, 0);
    assert_eq!(result.image_count, 0);
    assert_eq!(result.video_length_seconds, Decimal::ZERO);
    assert_eq!(result.audio_length_seconds, Decimal::ZERO);
}

#[test]
fn billable_input_tokens_subtracts_cache_hits_without_clamping() {
    let present = PromptModalityUsage {
        cached_tokens: Some(50),
        ..empty_usage()
    };
    let billable = get_billable_input_tokens(10, &present).expect("no overflow");
    assert_eq!(billable, -40, "no zero clamp applies here");
}

#[test]
fn billable_input_tokens_ignores_cache_write_tokens() {
    let present = PromptModalityUsage {
        cached_tokens: Some(2),
        cache_write_tokens: Some(1_000),
        ..empty_usage()
    };
    let billable = get_billable_input_tokens(10, &present).expect("no overflow");
    assert_eq!(billable, 8);
}

#[test]
fn cache_creation_token_details_pass_through_when_present() {
    let present = PromptModalityUsage {
        cache_creation_token_details: Some(CacheCreationTokenDetails {
            ephemeral_5m_input_tokens: None,
            ephemeral_1h_input_tokens: None,
        }),
        ..empty_usage()
    };
    let result: PromptTokensDetailsResult = parse_prompt_tokens_details(&present).expect("ok");
    assert!(result.cache_creation_token_details.is_some());
}
