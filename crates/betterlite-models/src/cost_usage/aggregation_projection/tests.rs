#![cfg(test)]

use std::collections::BTreeMap;

use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use crate::usage_sum::Usage;

#[test]
fn cost_usage_round_trips() {
    let original = CostUsage {
        prompt_tokens: 5,
        completion_tokens: 7,
        total_tokens: 12,
        prompt_details: Some(PromptDetails {
            text_tokens: Some(3),
            audio_tokens: Some(0),
            image_tokens: None,
            video_tokens: Some(1),
            web_search_requests: Some(2),
        }),
        completion_details: Some(CompletionDetails {
            text_tokens: Some(4),
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
            reasoning_tokens: Some(3),
        }),
        cache_read_tokens: Some(9),
        numeric_extras: BTreeMap::new(),
    };

    let usage = Usage::from(&original);
    let projected = CostUsage::from(&usage);

    assert_eq!(projected, original);
}
