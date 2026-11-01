#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde_json::Number;

use super::{completion_cost, completion_tokens_details};
use crate::cost_usage::{CompletionDetails, CostUsage};
use crate::modality_cost::completion::CompletionTokensDetailsResult;
use crate::tiered_pricing::Scalar;
use crate::token_rates::{EffectiveRates, ModelInfo};

fn base_usage() -> CostUsage {
    CostUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn no_completion_details() -> CompletionDetails {
    CompletionDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        reasoning_tokens: None,
    }
}

fn usage_without_details(completion_tokens: i64) -> CostUsage {
    CostUsage {
        completion_tokens,
        ..base_usage()
    }
}

fn rates() -> EffectiveRates {
    EffectiveRates {
        output_rate: Decimal::new(2, 0),
        reasoning_rate: Decimal::new(3, 0),
        input_rate: Decimal::ZERO,
        cache_read_rate: Decimal::ZERO,
        cache_creation_rate: Decimal::ZERO,
        cache_creation_above_1hr_rate: Decimal::ZERO,
    }
}

#[test]
fn absent_completion_details_become_all_text_and_suppress_modality_charging() {
    let usage = usage_without_details(7);
    let (result, is_text_tokens_total) = completion_tokens_details(&usage).expect("no overflow");
    assert_eq!(result.text_tokens, 7);
    assert!(is_text_tokens_total);
}

#[test]
fn a_zero_breakdown_with_zero_text_also_becomes_all_text() {
    let usage = CostUsage {
        completion_tokens: 9,
        completion_details: Some(no_completion_details()),
        ..base_usage()
    };
    let (result, is_text_tokens_total) = completion_tokens_details(&usage).expect("no overflow");
    assert_eq!(result.text_tokens, 9);
    assert!(is_text_tokens_total);
}

#[test]
fn an_explicit_nonzero_text_is_kept_without_suppressing_modality_charging() {
    let usage = CostUsage {
        completion_tokens: 10,
        completion_details: Some(CompletionDetails {
            text_tokens: Some(4),
            ..no_completion_details()
        }),
        ..base_usage()
    };
    let (result, is_text_tokens_total) = completion_tokens_details(&usage).expect("no overflow");
    assert_eq!(result.text_tokens, 4);
    assert!(!is_text_tokens_total);
}

#[test]
fn a_positive_breakdown_with_zero_text_computes_a_clamped_residual() {
    let usage = CostUsage {
        completion_tokens: 10,
        completion_details: Some(CompletionDetails {
            reasoning_tokens: Some(3),
            audio_tokens: Some(2),
            ..no_completion_details()
        }),
        ..base_usage()
    };
    let (result, is_text_tokens_total) = completion_tokens_details(&usage).expect("no overflow");
    assert_eq!(result.text_tokens, 5, "10 - 3 reasoning - 2 audio");
    assert!(!is_text_tokens_total);
}

fn details(
    text: i64,
    audio: i64,
    reasoning: i64,
    image: i64,
    video: i64,
) -> CompletionTokensDetailsResult {
    CompletionTokensDetailsResult {
        audio_tokens: audio,
        text_tokens: text,
        reasoning_tokens: reasoning,
        image_tokens: image,
        video_tokens: video,
    }
}

#[test]
fn text_total_mode_charges_only_the_output_rate() {
    let cost = completion_cost(&details(10, 5, 5, 5, 5), true, &ModelInfo::new(), &rates())
        .expect("no overflow");
    assert_eq!(
        cost,
        Decimal::new(20, 0),
        "5 of each modality are ignored while suppressed"
    );
}

#[test]
fn breakdown_mode_charges_each_modality_at_its_own_rate_or_the_fallback() {
    let cost = completion_cost(&details(2, 3, 1, 0, 0), false, &ModelInfo::new(), &rates())
        .expect("no overflow");
    // text: 2*2=4, audio falls back to output_rate: 3*2=6, reasoning: 1*3=3. image/video are zero.
    assert_eq!(cost, Decimal::new(13, 0));
}

#[test]
fn breakdown_mode_prefers_a_dedicated_modality_price_over_the_fallback() {
    let mut model = ModelInfo::new();
    model.insert(
        "output_cost_per_audio_token".to_owned(),
        Scalar::Number(Number::from_f64(5.0).expect("finite")),
    );
    let cost =
        completion_cost(&details(0, 2, 0, 0, 0), false, &model, &rates()).expect("no overflow");
    assert_eq!(
        cost,
        Decimal::new(10, 0),
        "2 audio tokens at the dedicated rate of 5"
    );
}

#[test]
fn breakdown_mode_ignores_nonpositive_modality_counts() {
    let cost = completion_cost(&details(4, -1, 0, 0, 0), false, &ModelInfo::new(), &rates())
        .expect("no overflow");
    assert_eq!(
        cost,
        Decimal::new(8, 0),
        "a negative audio count adds no charge"
    );
}
