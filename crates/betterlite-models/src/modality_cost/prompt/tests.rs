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
fn a_short_cache_hit_reaches_audio_before_text_and_image() {
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
    let result = parse_prompt_tokens_details(&usage(base, 5, details)).expect("no overflow");
    assert_eq!(result.cache_hit_audio_tokens, 3);
    assert_eq!(result.audio_tokens, 2);
    assert_eq!(result.text_tokens, 8);
    assert_eq!(result.image_tokens, 4);
}

#[rstest]
#[case::text(PromptDetails { text_tokens: Some(2), ..empty_base() }, CachedTokenCounters { text_tokens: Some(9), ..empty_counters() }, 0, 0, 0)]
#[case::audio(PromptDetails { audio_tokens: Some(2), ..empty_base() }, CachedTokenCounters { audio_tokens: Some(9), ..empty_counters() }, 0, 0, 0)]
#[case::image(PromptDetails { image_tokens: Some(2), ..empty_base() }, CachedTokenCounters { image_tokens: Some(9), ..empty_counters() }, 0, 0, 0)]
fn each_ordinary_modality_clamps_at_zero_when_its_cache_share_exceeds_it(
    #[case] base: PromptDetails,
    #[case] details: CachedTokenCounters,
    #[case] expected_text: i64,
    #[case] expected_audio: i64,
    #[case] expected_image: i64,
) {
    let result = parse_prompt_tokens_details(&usage(base, 9, details)).expect("no overflow");
    assert_eq!(result.text_tokens, expected_text);
    assert_eq!(result.audio_tokens, expected_audio);
    assert_eq!(result.image_tokens, expected_image);
}

#[test]
fn a_negative_cached_text_share_lifts_a_negative_reported_text_count() {
    let base = PromptDetails {
        text_tokens: Some(-2),
        ..empty_base()
    };
    let details = CachedTokenCounters {
        text_tokens: Some(-5),
        ..empty_counters()
    };
    let result = parse_prompt_tokens_details(&usage(base, 0, details)).expect("ok");
    assert_eq!(result.text_tokens, 3);
}

#[rstest]
#[case::absent_defaults_to_zero(None, None, 0, 0)]
#[case::zero_stays_zero(Some(0), Some(0), 0, 0)]
#[case::negative_collapses_to_zero(Some(-3), Some(-5), 0, 0)]
#[case::positive_is_kept(Some(4), Some(7), 4, 7)]
fn video_and_query_counts_require_a_strictly_positive_count(
    #[case] video: Option<i64>,
    #[case] query: Option<i64>,
    #[case] expected_video: i64,
    #[case] expected_query: i64,
) {
    let base = PromptDetails {
        video_tokens: video,
        ..empty_base()
    };
    let present = PromptModalityUsage {
        query_count: query,
        ..usage(base, 0, empty_counters())
    };
    let result = parse_prompt_tokens_details(&present).expect("ok");
    assert_eq!(result.video_tokens, expected_video);
    assert_eq!(result.query_count, expected_query);
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
fn negative_character_image_and_duration_fields_keep_their_sign() {
    let present = PromptModalityUsage {
        character_count: Some(-2),
        image_count: Some(-1),
        audio_length_seconds: Some(Decimal::new(-15, 1)),
        video_length_seconds: Some(Decimal::new(-25, 1)),
        ..empty_usage()
    };
    let result = parse_prompt_tokens_details(&present).expect("ok");
    assert_eq!(result.character_count, -2);
    assert_eq!(result.image_count, -1);
    assert_eq!(result.audio_length_seconds, Decimal::new(-15, 1));
    assert_eq!(result.video_length_seconds, Decimal::new(-25, 1));
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
fn cache_creation_ttl_buckets_pass_through_with_their_counts() {
    let present = PromptModalityUsage {
        cache_creation_token_details: Some(CacheCreationTokenDetails {
            ephemeral_5m_input_tokens: Some(3),
            ephemeral_1h_input_tokens: Some(4),
        }),
        ..empty_usage()
    };
    let result: PromptTokensDetailsResult = parse_prompt_tokens_details(&present).expect("ok");
    assert_eq!(
        result.cache_creation_token_details,
        present.cache_creation_token_details
    );
}
