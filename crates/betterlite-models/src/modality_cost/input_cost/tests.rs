#![cfg(test)]

use rust_decimal::Decimal;

use super::{InputCostRates, calculate_input_cost};
use crate::modality_cost::prompt::PromptTokensDetailsResult;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

fn price(text: &str) -> Scalar {
    Scalar::Text(text.to_owned())
}

fn rates() -> InputCostRates {
    InputCostRates {
        prompt_base_cost: Decimal::from(1_i64),
        cache_read_cost: Decimal::new(5, 1),
        cache_creation_cost: Decimal::from(2_i64),
        cache_creation_cost_above_1hr: Decimal::from(4_i64),
    }
}

fn details() -> PromptTokensDetailsResult {
    PromptTokensDetailsResult {
        cache_hit_tokens: 0,
        cache_hit_audio_tokens: 0,
        cache_creation_tokens: 0,
        cache_creation_token_details: None,
        text_tokens: 10,
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

#[test]
fn text_tokens_bill_at_the_base_rate_with_no_other_usage() {
    let model = ModelInfo::new();
    let cost = calculate_input_cost(&details(), &model, &rates(), None).expect("no overflow");
    assert_eq!(cost, Decimal::from(10_i64));
}

#[test]
fn cached_audio_bills_at_the_service_aware_audio_read_rate_and_the_rest_at_the_ordinary_rate() {
    let mut model = ModelInfo::new();
    model.insert("cache_read_input_audio_token_cost".to_owned(), price("0.1"));
    let usage = PromptTokensDetailsResult {
        cache_hit_tokens: 10,
        cache_hit_audio_tokens: 4,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    // text: 10*1 = 10; non-audio cache hit: 6*0.5 = 3; audio cache hit: 4*0.1 = 0.4
    assert_eq!(cost, Decimal::new(134, 1));
}

#[test]
fn cached_audio_falls_back_to_the_ordinary_cache_read_rate_when_no_audio_rate_is_set() {
    let model = ModelInfo::new();
    let usage = PromptTokensDetailsResult {
        cache_hit_tokens: 10,
        cache_hit_audio_tokens: 4,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    // text: 10; all 10 cache-hit tokens bill at the ordinary 0.5 read rate: 5
    assert_eq!(cost, Decimal::from(15_i64));
}

#[test]
fn audio_tokens_bill_at_the_audio_rate_when_no_per_second_price_is_set() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_audio_token".to_owned(), price("2"));
    let usage = PromptTokensDetailsResult {
        audio_tokens: 3,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    assert_eq!(cost, Decimal::from(16_i64));
}

#[test]
fn a_per_second_audio_price_with_positive_seconds_suppresses_the_audio_token_charge() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_audio_token".to_owned(), price("2"));
    model.insert("input_cost_per_audio_per_second".to_owned(), price("1"));
    let usage = PromptTokensDetailsResult {
        audio_tokens: 3,
        audio_length_seconds: Decimal::from(5_i64),
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    // audio token charge (6) is suppressed; only the per-second component (5*1=5) is added.
    assert_eq!(cost, Decimal::from(15_i64));
}

#[test]
fn image_tokens_fall_back_to_the_generic_token_price_when_no_image_token_price_is_set() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_token".to_owned(), price("3"));
    let usage = PromptTokensDetailsResult {
        image_tokens: 2,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    assert_eq!(cost, Decimal::from(16_i64));
}

#[test]
fn a_per_image_price_with_a_positive_image_count_suppresses_the_image_token_charge() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_image_token".to_owned(), price("3"));
    model.insert("input_cost_per_image".to_owned(), price("1"));
    let usage = PromptTokensDetailsResult {
        image_tokens: 2,
        image_count: 1,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    // image token charge (6) is suppressed; the per-image component (1*1=1) is added.
    assert_eq!(cost, Decimal::from(11_i64));
}

#[test]
fn cache_creation_tokens_add_the_write_cost_at_the_ordinary_rate() {
    let model = ModelInfo::new();
    let usage = PromptTokensDetailsResult {
        cache_creation_tokens: 5,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    assert_eq!(cost, Decimal::from(20_i64));
}

#[test]
fn character_image_duration_and_query_components_are_all_added() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_character".to_owned(), price("1"));
    model.insert("input_cost_per_query".to_owned(), price("1"));
    let usage = PromptTokensDetailsResult {
        character_count: 3,
        query_count: 2,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), None).expect("no overflow");
    assert_eq!(cost, Decimal::from(15_i64));
}

#[test]
fn negative_image_count_and_audio_seconds_still_suppress_their_token_charges() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_image".to_owned(), price("1"));
    model.insert("input_cost_per_image_token".to_owned(), price("0.5"));
    model.insert("input_cost_per_audio_per_second".to_owned(), price("1"));
    model.insert("input_cost_per_audio_token".to_owned(), price("0.25"));
    let usage = PromptTokensDetailsResult {
        audio_tokens: 2,
        image_tokens: 4,
        image_count: -1,
        audio_length_seconds: Decimal::new(-15, 1),
        ..details()
    };
    let cost = calculate_input_cost(
        &usage,
        &model,
        &InputCostRates {
            prompt_base_cost: Decimal::new(5, 1),
            cache_read_cost: Decimal::ZERO,
            cache_creation_cost: Decimal::ZERO,
            cache_creation_cost_above_1hr: Decimal::ZERO,
        },
        None,
    )
    .expect("no overflow");
    assert_eq!(cost, Decimal::from(5_i64));
}

#[test]
fn video_tokens_fall_back_to_the_generic_token_price_when_no_video_token_price_is_set() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_token".to_owned(), price("0.5"));
    let usage = PromptTokensDetailsResult {
        video_tokens: 4,
        ..details()
    };
    let cost = calculate_input_cost(
        &usage,
        &model,
        &InputCostRates {
            prompt_base_cost: Decimal::new(5, 1),
            cache_read_cost: Decimal::ZERO,
            cache_creation_cost: Decimal::ZERO,
            cache_creation_cost_above_1hr: Decimal::ZERO,
        },
        None,
    )
    .expect("no overflow");
    assert_eq!(cost, Decimal::from(7_i64));
}

#[test]
fn a_free_per_second_video_price_with_positive_seconds_suppresses_the_video_token_charge() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_video_token".to_owned(), price("0.5"));
    model.insert("input_cost_per_video_per_second".to_owned(), price("0"));
    let usage = PromptTokensDetailsResult {
        video_tokens: 4,
        video_length_seconds: Decimal::from(3_i64),
        ..details()
    };
    let cost = calculate_input_cost(
        &usage,
        &model,
        &InputCostRates {
            prompt_base_cost: Decimal::new(5, 1),
            cache_read_cost: Decimal::ZERO,
            cache_creation_cost: Decimal::ZERO,
            cache_creation_cost_above_1hr: Decimal::ZERO,
        },
        None,
    )
    .expect("no overflow");
    assert_eq!(cost, Decimal::from(5_i64));
}

#[test]
fn video_seconds_bill_at_the_per_second_price() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_video_token".to_owned(), price("0.5"));
    model.insert("input_cost_per_video_per_second".to_owned(), price("0.25"));
    let usage = PromptTokensDetailsResult {
        video_tokens: 4,
        video_length_seconds: Decimal::from(3_i64),
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &modality_rates(), None).expect("no overflow");
    assert_eq!(cost, Decimal::new(575, 2));
}

#[test]
fn flex_tier_prices_cached_audio_and_audio_tokens_at_their_flex_keys() {
    let mut model = ModelInfo::new();
    model.insert("cache_read_input_audio_token_cost".to_owned(), price("0.5"));
    model.insert(
        "cache_read_input_audio_token_cost_flex".to_owned(),
        price("0.25"),
    );
    model.insert("input_cost_per_audio_token".to_owned(), price("1"));
    model.insert("input_cost_per_audio_token_flex".to_owned(), price("0.5"));
    let usage = PromptTokensDetailsResult {
        cache_hit_tokens: 4,
        cache_hit_audio_tokens: 4,
        audio_tokens: 6,
        ..details()
    };
    let cost = calculate_input_cost(&usage, &model, &rates(), Some("flex")).expect("no overflow");
    assert_eq!(cost, Decimal::from(14_i64));
}
