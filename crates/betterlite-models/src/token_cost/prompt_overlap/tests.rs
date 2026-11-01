#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{PromptExtras, prompt_tokens_details, repair_prompt_overlap};
use crate::cost_usage::{CostUsage, PromptDetails};
use crate::modality_cost::prompt::PromptTokensDetailsResult;

fn usage_without_details(prompt_tokens: i64) -> CostUsage {
    CostUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn empty_prompt_details() -> PromptDetails {
    PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
    }
}

fn empty_extras() -> PromptExtras {
    PromptExtras {
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

fn all_zero_details() -> PromptTokensDetailsResult {
    PromptTokensDetailsResult {
        cache_hit_tokens: 0,
        cache_hit_audio_tokens: 0,
        cache_creation_tokens: 0,
        cache_creation_token_details: None,
        text_tokens: 0,
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        character_count: 0,
        image_count: 0,
        video_length_seconds: Decimal::ZERO,
        audio_length_seconds: Decimal::ZERO,
        query_count: 0,
    }
}

#[derive(Clone, Copy)]
struct DetailsFixture {
    text: i64,
    cache_hit: i64,
    cache_creation: i64,
    audio: i64,
    image: i64,
    video: i64,
    image_count: i64,
}

fn details(fixture: DetailsFixture) -> PromptTokensDetailsResult {
    PromptTokensDetailsResult {
        cache_hit_tokens: fixture.cache_hit,
        cache_creation_tokens: fixture.cache_creation,
        text_tokens: fixture.text,
        audio_tokens: fixture.audio,
        image_tokens: fixture.image,
        video_tokens: fixture.video,
        image_count: fixture.image_count,
        ..all_zero_details()
    }
}

#[test]
fn absent_prompt_details_become_all_text() {
    let usage = usage_without_details(42);
    let result = prompt_tokens_details(&usage, None).expect("no overflow");
    assert_eq!(result.text_tokens, 42);
    assert_eq!(result.audio_tokens, 0);
    assert_eq!(result.image_tokens, 0);
}

#[test]
fn present_prompt_details_route_through_extras() {
    let usage = CostUsage {
        prompt_details: Some(PromptDetails {
            image_tokens: Some(6),
            ..empty_prompt_details()
        }),
        ..usage_without_details(10)
    };
    let extras = PromptExtras {
        image_count: Some(1),
        ..empty_extras()
    };
    let result = prompt_tokens_details(&usage, Some(&extras)).expect("no overflow");
    assert_eq!(result.image_tokens, 6);
    assert_eq!(result.image_count, 1);
}

#[test]
fn overlap_allocates_audio_before_image_before_video_then_text() {
    // prompt_tokens=10, cache_hit=4: budget is 6 for audio/image/video/text combined.
    let parsed = details(DetailsFixture {
        text: 5,
        cache_hit: 4,
        cache_creation: 0,
        audio: 3,
        image: 2,
        video: 2,
        image_count: 0,
    });
    let result = repair_prompt_overlap(parsed, 10).expect("no overflow");
    assert_eq!(result.audio_tokens, 3);
    assert_eq!(result.image_tokens, 2);
    assert_eq!(
        result.video_tokens, 1,
        "only one slot remains after audio and image"
    );
    assert_eq!(result.text_tokens, 0);
}

#[test]
fn overlap_repair_only_triggers_with_a_positive_cache_amount() {
    // text+audio+image+video exceeds prompt_tokens, but cache_hit and cache_creation are both zero.
    let parsed = details(DetailsFixture {
        text: 5,
        cache_hit: 0,
        cache_creation: 0,
        audio: 3,
        image: 2,
        video: 2,
        image_count: 0,
    });
    let result = repair_prompt_overlap(parsed, 10).expect("no overflow");
    assert_eq!(
        result.text_tokens, 5,
        "unchanged because nothing is double counted"
    );
    assert_eq!(result.audio_tokens, 3);
}

#[test]
fn zero_text_with_zero_image_count_infers_the_residual() {
    let parsed = details(DetailsFixture {
        text: 0,
        cache_hit: 2,
        cache_creation: 0,
        audio: 1,
        image: 0,
        video: 0,
        image_count: 0,
    });
    let result = repair_prompt_overlap(parsed, 10).expect("no overflow");
    assert_eq!(result.text_tokens, 7, "10 - 2 cache hit - 1 audio");
}

#[test]
fn a_positive_image_count_blocks_the_text_residual_inference() {
    let parsed = details(DetailsFixture {
        text: 0,
        cache_hit: 2,
        cache_creation: 0,
        audio: 1,
        image: 0,
        video: 0,
        image_count: 3,
    });
    let result = repair_prompt_overlap(parsed, 10).expect("no overflow");
    assert_eq!(
        result.text_tokens, 0,
        "image_count > 0 deliberately blocks inference"
    );
}

#[test]
fn nonzero_text_without_overlap_is_left_as_reported() {
    let parsed = details(DetailsFixture {
        text: 5,
        cache_hit: 0,
        cache_creation: 0,
        audio: 0,
        image: 0,
        video: 0,
        image_count: 0,
    });
    let result = repair_prompt_overlap(parsed, 5).expect("no overflow");
    assert_eq!(result.text_tokens, 5);
}
