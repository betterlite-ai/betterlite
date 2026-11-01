#![cfg(test)]

use rust_decimal::Decimal;

use super::calculate_cache_writing_cost;
use crate::cost_usage::modality_projection::CacheCreationTokenDetails;

#[test]
fn absent_details_bills_the_aggregate_at_the_ordinary_write_price() {
    let cost = calculate_cache_writing_cost(10, None, Decimal::from(2_i64), Decimal::from(5_i64))
        .expect("no overflow");
    assert_eq!(cost, Decimal::from(20_i64));
}

#[test]
fn present_details_replace_the_aggregate_even_when_both_ttl_fields_are_null() {
    let details = CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: None,
        ephemeral_1h_input_tokens: None,
    };
    let cost = calculate_cache_writing_cost(
        10,
        Some(details),
        Decimal::from(2_i64),
        Decimal::from(5_i64),
    )
    .expect("no overflow");
    assert_eq!(
        cost,
        Decimal::ZERO,
        "the aggregate is ignored once the TTL record is present"
    );
}

#[test]
fn present_details_replace_the_aggregate_even_when_both_ttl_fields_are_zero() {
    let details = CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(0),
        ephemeral_1h_input_tokens: Some(0),
    };
    let cost = calculate_cache_writing_cost(
        10,
        Some(details),
        Decimal::from(2_i64),
        Decimal::from(5_i64),
    )
    .expect("no overflow");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn five_minute_and_one_hour_tokens_bill_at_their_own_price() {
    let details = CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(3),
        ephemeral_1h_input_tokens: Some(4),
    };
    let cost = calculate_cache_writing_cost(
        999,
        Some(details),
        Decimal::from(2_i64),
        Decimal::from(5_i64),
    )
    .expect("no overflow");
    assert_eq!(cost, Decimal::from(3_i64 * 2 + 4 * 5));
}
