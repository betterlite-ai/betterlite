#![cfg(test)]

use super::{PromptModalityUsage, resolved_cache_creation_tokens};
use crate::cost_usage::PromptDetails;

fn base() -> PromptDetails {
    PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
    }
}

fn usage() -> PromptModalityUsage {
    PromptModalityUsage {
        base: base(),
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

#[test]
fn nonzero_cache_write_tokens_wins_over_cache_creation_tokens() {
    let present = PromptModalityUsage {
        cache_write_tokens: Some(5),
        cache_creation_tokens: Some(9),
        ..usage()
    };
    assert_eq!(resolved_cache_creation_tokens(&present), 5);
}

#[test]
fn explicit_zero_cache_write_tokens_falls_through_to_cache_creation_tokens() {
    let present = PromptModalityUsage {
        cache_write_tokens: Some(0),
        cache_creation_tokens: Some(9),
        ..usage()
    };
    assert_eq!(resolved_cache_creation_tokens(&present), 9);
}

#[test]
fn absent_cache_write_tokens_falls_through_to_cache_creation_tokens() {
    let present = PromptModalityUsage {
        cache_creation_tokens: Some(9),
        ..usage()
    };
    assert_eq!(resolved_cache_creation_tokens(&present), 9);
}

#[test]
fn both_absent_default_to_zero() {
    assert_eq!(resolved_cache_creation_tokens(&usage()), 0);
}
